//! Fixture-based tests comparing pglet output to expected results.
//!
//! Self-contained: builds server, spawns on open port, runs fixtures, cleans up.
//!
//! Test mode (default):
//!   cargo run -p `pglet_tests` --bin `test_fixtures`
//!   cargo run -p `pglet_tests` --bin `test_fixtures` -- select/
//!   cargo run -p `pglet_tests` --bin `test_fixtures` -- -v
//!
//! Update mode (regenerates expected from Postgres):
//!   cargo run -p `pglet_tests` --bin `test_fixtures` -- --update
//!   cargo run -p `pglet_tests` --bin `test_fixtures` -- --update select/
//!
//! Profile-lock mode (the stripped-build regression lock — run under
//! `PGLET_NO_INDEX_ACCEL=1`, which makes the spawned server `--no-default-features`):
//!   cargo run -p `pglet_tests` --bin `test_fixtures` -- --profile-lock

// These futures hold non-`Send` bound-param boxes (`dyn ToSql + Sync`) across awaits.
// This is a test-runner binary on a local tokio runtime — the futures never cross a
// thread boundary, so `future_not_send` doesn't apply.
#![allow(clippy::future_not_send)]

use pglet_testing::server::stripped_profile_requested;
use pglet_tests::{
    DbConfig, DeclaredTypes, Expected, FixtureParseError, POSTGRES_CONFIG, ParsedFixture,
    QueryError, SimpleProbe, connect, ensure_postgres, is_client_fault, oracle_identity_is_default,
    parse_fixture, pglet_config, run_query_catching_errors, run_query_dual, run_text_types,
    spawn_test_server, test_is_read_only,
};
use std::fs;
use std::path::{Path, PathBuf};
use tokio_postgres::Client;

// =============================================================================
// Fixture Discovery
// =============================================================================

fn discover_fixtures(dir: &Path) -> Vec<PathBuf> {
    let mut fixtures = Vec::new();
    discover_recursive(dir, &mut fixtures);
    fixtures.sort();
    fixtures
}

fn discover_recursive(dir: &Path, fixtures: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                discover_recursive(&path, fixtures);
            } else if path.extension().is_some_and(|e| e == "sql") {
                fixtures.push(path);
            }
        }
    }
}

fn expected_path(sql_path: &Path) -> PathBuf {
    sql_path.with_extension("expected.json")
}

/// Reset the Postgres `public` schema so fixtures start from a clean slate.
///
/// The harness isolates each fixture in a rolled-back transaction, so fixtures
/// never persist tables — but a *shared* Postgres can still carry leftovers from
/// older (pre-isolation) runs or a crash. Dropping and recreating `public` once
/// at startup makes the suite self-contained and is what lets fixtures omit the
/// per-file `DROP TABLE IF EXISTS` boilerplate. pglet needs no reset — its test
/// server is spawned fresh (in-memory) each run and an isolated fixture never
/// commits — and it doesn't implement `DROP SCHEMA` anyway.
async fn reset_postgres_schema() -> Result<(), String> {
    let client = connect(&POSTGRES_CONFIG)
        .await
        .map_err(|e| format!("schema-reset connection failed: {e}"))?;
    client
        .simple_query("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .await
        .map_err(|e| format!("schema reset (DROP/CREATE SCHEMA public) failed: {e}"))?;
    Ok(())
}

// =============================================================================
// Test Mode
// =============================================================================

/// How many fixtures one declared-type leg actually asserted on, versus the ways it
/// legitimately had nothing to say — and the ways it had something bad to say.
///
/// Reported in the run summary because the alternative is a figure in a doc, and a
/// figure in a doc drifts the moment a fixture is added. It is also the check's own
/// smoke alarm: `asserted` collapsing toward zero is what a silently-broken probe
/// looks like, and a bare green bar would not show it.
///
/// Every eligible fixture lands in exactly one bucket or in the derived `not_reached`
/// (it failed an earlier comparison, so the check never ran). Without that residual
/// the printed total silently under-reports by however many fixtures went red —
/// precisely when the number matters most.
#[derive(Default)]
struct DeclaredTypeCoverage {
    /// Fixtures whose result this leg can read at all (see `has_*_leg`).
    eligible: usize,
    /// Fixtures with no leg of this kind (`@expect-error`, `@param`/`@no-txn` on the
    /// text leg, `@text` on the extended leg, a `@no-declared-types:` opt-out).
    not_eligible: usize,
    /// Compared a non-empty column list on both sides, and they agreed.
    asserted: usize,
    /// Ran, but neither side described a row (an `INSERT`/DDL `@test`).
    no_columns: usize,
    /// Ran, but the `@test` errored identically on both engines.
    both_errored: usize,
    /// Ran and found a divergence — a red fixture.
    mismatched: usize,
    /// Could not run: the read itself failed on at least one side — a red fixture.
    probe_failed: usize,
}

impl DeclaredTypeCoverage {
    /// Print what this leg's declared-type check reached, under `leg` as its heading.
    fn report(&self, leg: &str) {
        let total = self.eligible + self.not_eligible;
        // Every eligible fixture the check never reached, because an earlier
        // comparison (stale expected, values, text format) already failed it.
        let not_reached = self.eligible.saturating_sub(
            self.asserted
                + self.no_columns
                + self.both_errored
                + self.mismatched
                + self.probe_failed,
        );
        println!("\n=== Declared column types ({leg}) ===");
        println!("Asserted:     {} of {total} fixtures", self.asserted);
        println!(
            "No assertion: {} eligible ({} describe no row, {} error on both engines, \
             {not_reached} never reached), {} not eligible",
            self.no_columns + self.both_errored + not_reached,
            self.no_columns,
            self.both_errored,
            self.not_eligible
        );
        println!(
            "Divergent:    {} mismatched, {} unreadable",
            self.mismatched, self.probe_failed
        );
    }

    /// Fold one fixture's outcome into the tally, returning the failure line to print
    /// when the check went red (`None` when it did not).
    fn record(&mut self, outcome: DeclaredTypes) -> Option<String> {
        match outcome {
            DeclaredTypes::NotEligible => None,
            // `Match` implies both sides listed the same non-empty columns:
            // two empty lists return `NoColumns` before the diff runs.
            DeclaredTypes::Match => {
                self.asserted += 1;
                None
            }
            DeclaredTypes::NoColumns => {
                self.no_columns += 1;
                None
            }
            DeclaredTypes::BothErrored(_) => {
                self.both_errored += 1;
                None
            }
            DeclaredTypes::Mismatch(diff) => {
                self.mismatched += 1;
                Some(format!("declared column types: {diff}"))
            }
            DeclaredTypes::ProbeFailed(why) => {
                self.probe_failed += 1;
                Some(format!("declared column types unchecked: {why}"))
            }
        }
    }
}

async fn run_test_mode(
    fixtures_dir: &Path,
    fixtures: &[PathBuf],
    verbose: bool,
) -> (usize, usize, usize) {
    // Ensure Postgres is running (for validation)
    if let Err(e) = ensure_postgres().await {
        eprintln!("[fixtures] {e}");
        std::process::exit(1);
    }
    // Said up front, because the failure it causes reads as a corpus regression:
    // a fixture that selects `current_database()` compares against a snapshot
    // holding the *default* spelling, and reports "expected file is stale".
    if !oracle_identity_is_default(&POSTGRES_CONFIG) {
        eprintln!(
            "[fixtures] NOTE: the oracle is user `{}` on database `{}`, not the default \
             `postgres`/`postgres`. Fixtures that select the session identity \
             (`current_user`, `current_database()`) record the oracle's own spelling, so \
             they will report their expected file as stale. That is the override, not a \
             regression.",
            POSTGRES_CONFIG.user, POSTGRES_CONFIG.database
        );
    }
    // Clean slate: reset Postgres's public schema so leftover tables from older
    // (pre-isolation) runs can't collide with fixtures that no longer DROP-first.
    if let Err(e) = reset_postgres_schema().await {
        eprintln!("[fixtures] {e}");
        std::process::exit(1);
    }

    // Spawn a fresh pglet server (built from current source, on an open port) — never
    // reuse a leftover one, which could be a stale binary or carry another run's residue.
    eprintln!("[fixtures] Starting pglet server...");
    let server_handle = match spawn_test_server().await {
        Ok(handle) => {
            eprintln!("[fixtures] Started pglet server on port {}", handle.port);
            handle
        }
        Err(e) => {
            eprintln!("[fixtures] Failed to start pglet server: {e}");
            std::process::exit(1);
        }
    };
    let pglet_cfg = pglet_config(server_handle.port);

    println!("\n=== Fixture Tests ===\n");
    println!("Running {} fixtures...\n", fixtures.len());

    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    // Two legs, two tallies: pglet computes its declared column types through
    // different code per protocol, so they are separate assertions over separate
    // corpora (a `@param` fixture has only an extended leg, a `@text` one only a text
    // leg) and folding them into one number would hide which walker was checked.
    let mut text_types = DeclaredTypeCoverage::default();
    let mut extended_types = DeclaredTypeCoverage::default();

    for fixture_path in fixtures {
        let rel_path = fixture_path
            .strip_prefix(fixtures_dir)
            .unwrap_or(fixture_path);
        let name = rel_path.to_string_lossy();

        // Read and parse fixture
        let content = match fs::read_to_string(fixture_path) {
            Ok(c) => c,
            Err(e) => {
                println!("\x1b[33m?\x1b[0m {name}: read error: {e}");
                skipped += 1;
                continue;
            }
        };

        // A fixture from another directive family (the `copy/` suite) is skipped —
        // it isn't this binary's to run. A *malformed* one is a defect and fails:
        // absorbing it as a skip is how a typo'd directive stays green.
        let parsed: ParsedFixture = match parse_fixture(&content, &name) {
            Ok(p) => p,
            Err(FixtureParseError::ForeignFamily(e)) => {
                println!("\x1b[33m?\x1b[0m {e}");
                skipped += 1;
                continue;
            }
            Err(FixtureParseError::Invalid(e)) => {
                println!("\x1b[31m✗\x1b[0m {e}");
                failed += 1;
                continue;
            }
        };

        // Read expected
        let exp_path = expected_path(fixture_path);
        let Ok(expected_content) = fs::read_to_string(&exp_path) else {
            println!("\x1b[33m?\x1b[0m {name}: missing .expected.json (run --update)");
            skipped += 1;
            continue;
        };

        let expected: Expected = match serde_json::from_str(&expected_content) {
            Ok(e) => e,
            Err(e) => {
                println!("\x1b[33m?\x1b[0m {name}: expected parse error: {e}");
                skipped += 1;
                continue;
            }
        };

        // Connect both servers.
        let pg_client = match connect(&POSTGRES_CONFIG).await {
            Ok(c) => c,
            Err(e) => {
                println!("\x1b[31m✗\x1b[0m {name}: Postgres connection failed: {e}");
                failed += 1;
                continue;
            }
        };
        let pglet_client = match connect(&pglet_cfg).await {
            Ok(c) => c,
            Err(e) => {
                println!("\x1b[31m✗\x1b[0m {name}: pglet connection failed: {e}");
                failed += 1;
                continue;
            }
        };
        // Capture each server's result(s): the binary (or `@text`) result, plus the
        // text result where the dual-format check applies. A read-only success
        // fixture captures both formats in a single transaction (one setup); a
        // mutating one re-runs setup for the text pass (see `capture`).
        let (pg_result, pg_text) = capture(&pg_client, &parsed).await;
        let (pglet_result, pglet_text) = capture(&pglet_client, &parsed).await;

        // Per-leg eligibility, decided and counted **here** — before any comparison
        // below can `continue` past it — so the printed totals cover the whole corpus
        // rather than only the fixtures that got far enough to be asserted on.
        let text_leg = has_text_leg(&parsed);
        let extended_leg = has_extended_leg(&parsed);
        for (eligible, tally) in [
            (text_leg, &mut text_types),
            (extended_leg, &mut extended_types),
        ] {
            if eligible {
                tally.eligible += 1;
            } else {
                tally.not_eligible += 1;
            }
        }

        // The declared column types of the text leg, over a hand-rolled simple-query
        // connection — the one thing tokio-postgres cannot report (see `SimpleProbe`).
        // The extended leg's need no such detour: they ride along on `capture` below,
        // read off the prepared statement's `Describe` reply.
        let (pg_types, pglet_types) = if text_leg {
            match connect_probes(&POSTGRES_CONFIG, &pglet_cfg).await {
                Ok((mut pg_probe, mut pglet_probe)) => (
                    capture_types(&mut pg_probe, &parsed).await,
                    capture_types(&mut pglet_probe, &parsed).await,
                ),
                Err(e) => {
                    println!("\x1b[31m✗\x1b[0m {name}: text-protocol connection failed: {e}");
                    failed += 1;
                    continue;
                }
            }
        } else {
            (
                Expected::success(Vec::new(), 0),
                Expected::success(Vec::new(), 0),
            )
        };

        // Step 1: the expected file must still match live Postgres (stale-detection).
        if !expected.values_match(&pg_result) {
            println!("\x1b[31m✗\x1b[0m {name}: expected file is stale (run --update)");
            if verbose {
                println!("  Postgres: {pg_result:?}");
                println!("  Expected: {expected:?}");
            }
            failed += 1;
            continue;
        }

        // Step 2: Compare pglet to expected (+ the dual text format where captured).
        if parsed.expect_error.is_some() {
            // @expect-error fixture: pglet must error. The recorded `expected`
            // (regenerated from Postgres) auto-detects the assertion strength:
            //
            // - `expected` is an Error → Postgres errored too, so pglet's
            //   SQLSTATE must match Postgres's recorded `code` (catches a wrong
            //   23505-vs-23503-vs-42704 that "any error passes" would miss).
            // - `expected` is a Success → Postgres ACCEPTED the query (the
            //   deliberately pglet-stricter-than-PG cases, e.g.
            //   `types/unsupported_unknown_column.sql`). Any pglet error passes.
            match (&pglet_result, &expected) {
                (Expected::Error { error: pglet_err }, Expected::Error { error: pg_err }) => {
                    if pglet_err.code == pg_err.code {
                        // The SQLSTATE agrees; now the **structured** fields pglet sent.
                        // Compared against the *live* Postgres error (`pg_result`), not
                        // the recorded one — they are a live assertion, exactly like
                        // the declared column types, and are never written to
                        // `.expected.json`.
                        match check_error_fields(
                            &parsed,
                            &pg_result,
                            &pglet_result,
                            pglet_err,
                            pg_err,
                        ) {
                            Ok(()) => {
                                println!(
                                    "\x1b[32m✓\x1b[0m {name} (expected error {})",
                                    pg_err.code
                                );
                                passed += 1;
                            }
                            Err(why) => {
                                println!("\x1b[31m✗\x1b[0m {name}: {why}");
                                failed += 1;
                            }
                        }
                    } else {
                        println!(
                            "\x1b[31m✗\x1b[0m {name}: SQLSTATE mismatch (Postgres {}, pglet {})",
                            pg_err.code, pglet_err.code
                        );
                        if verbose {
                            println!("  Postgres: {pg_err:?}");
                            println!("  pglet:    {pglet_err:?}");
                        }
                        failed += 1;
                    }
                }
                (Expected::Error { .. }, Expected::Success { .. }) => {
                    // Postgres accepted the query (pglet-stricter case); any pglet
                    // *engine* error passes — but a harness-side failure does not.
                    // A client decode error lands in the same slot and would
                    // otherwise score green off the very declared-vs-sent type
                    // divergence this suite exists to catch.
                    if is_client_fault(&pglet_result) {
                        println!(
                            "\x1b[31m✗\x1b[0m {name}: harness-side failure, not an engine error"
                        );
                        if verbose {
                            println!("  pglet: {pglet_result:?}");
                        }
                        failed += 1;
                    } else {
                        println!("\x1b[32m✓\x1b[0m {name} (expected error, pglet-stricter)");
                        passed += 1;
                    }
                }
                (Expected::Success { .. }, _) => {
                    println!("\x1b[31m✗\x1b[0m {name}: expected error but got success");
                    failed += 1;
                }
            }
        } else {
            // Normal fixture: compare pglet's (binary) result to expected.
            if !expected.values_match(&pglet_result) {
                println!("\x1b[31m✗\x1b[0m {name}");
                if verbose {
                    println!("  Expected: {expected:?}");
                    println!("  pglet:    {pglet_result:?}");
                }
                failed += 1;
                continue;
            }
            // Dual-format check: the binary path passed, so also compare the *text*
            // wire output (pglet-text vs Postgres-text). A text-encoder-only bug —
            // e.g. a `timestamptz` rendered without its `+00` offset — is invisible
            // to the binary path, so without this it would pass green unless a
            // fixture happened to opt into `@text`. The text results are captured by
            // `capture` (`None` where text can't or needn't run — `@text`/`@param`/
            // `@no-txn`/`@expect-error`).
            if let (Some(pg_text), Some(pglet_text)) = (&pg_text, &pglet_text)
                && !pg_text.values_match(pglet_text)
            {
                println!("\x1b[31m✗\x1b[0m {name} (text format)");
                if verbose {
                    println!("  Postgres(text): {pg_text:?}");
                    println!("  pglet(text):    {pglet_text:?}");
                }
                failed += 1;
                continue;
            }
            // Declared-type checks. The values above compared equal, which proves
            // less than it looks on either leg:
            //
            // - **text leg** — every value is a string over the simple query
            //   protocol, so an `integer` column declared `text` still sends `10` and
            //   still matches byte-for-byte. The `RowDescription` is the only place
            //   the two differ, and it is what every text-protocol driver types its
            //   output by.
            // - **extended leg** — a declared type outright incompatible with the
            //   bytes sent already fails the decode, but a **wrong yet
            //   width-compatible** one (`text` where Postgres says `int4[]`, `int8`
            //   where it says `int4`) decodes cleanly. That leg is a *different
            //   walker* (the planner's `Plan::result_shape`, vs the executor's
            //   `QueryResult::shape` behind the text leg), so agreement on one says
            //   nothing about the other — and the extended one is what a binary
            //   client decodes against.
            //
            // A read that failed is *not* a pass: folding it into "no divergence" is
            // how a decode bug becomes its own alibi (the same hole `is_client_fault`
            // closes on the `@expect-error` arm above), so it fails the fixture and
            // names what broke.
            //
            // Each leg is skipped outright where the fixture has none, so the empty
            // placeholders can't pose as a `NoColumns` agreement (already counted
            // `not_eligible` above).
            let legs = [
                (
                    "text-leg",
                    &mut text_types,
                    if text_leg {
                        pg_types.compare_declared_types(&pglet_types)
                    } else {
                        DeclaredTypes::NotEligible
                    },
                ),
                (
                    "extended-leg",
                    &mut extended_types,
                    if extended_leg {
                        pg_result.compare_declared_types(&pglet_result)
                    } else {
                        DeclaredTypes::NotEligible
                    },
                ),
            ];
            // Both legs are folded into their tallies before either can fail the
            // fixture, so one leg going red never suppresses the other's count.
            let mut failures = Vec::new();
            for (leg, tally, outcome) in legs {
                if let Some(why) = tally.record(outcome) {
                    failures.push(format!("{leg} {why}"));
                }
            }
            if !failures.is_empty() {
                println!("\x1b[31m✗\x1b[0m {name} ({})", failures.join("; "));
                failed += 1;
                continue;
            }
            println!("\x1b[32m✓\x1b[0m {name}");
            passed += 1;
        }
    }

    text_types.report("text leg — simple query, the executor walker");
    extended_types.report("extended leg — prepared Describe, the planner walker");

    (passed, failed, skipped)
}

/// Open the text-leg probe connection to each server.
async fn connect_probes(
    pg_cfg: &DbConfig,
    pglet_cfg: &DbConfig,
) -> Result<(SimpleProbe, SimpleProbe), String> {
    let pg = SimpleProbe::connect(pg_cfg)
        .await
        .map_err(|e| format!("Postgres: {e}"))?;
    let pglet = SimpleProbe::connect(pglet_cfg)
        .await
        .map_err(|e| format!("pglet: {e}"))?;
    Ok((pg, pglet))
}

/// Capture a fixture's result(s) against one server: the binary (or `@text`)
/// result, plus the text result when the dual-format check applies (`None`
/// otherwise). The dual check applies only to a plain success fixture in the
/// default format — no `@text`/`@param`/`@no-txn`/`@expect-error`. For a
/// **read-only** `@test` the two formats share one transaction (one setup); a
/// **mutating** `@test` re-runs setup in a separate txn for the text pass, because
/// running it twice in one txn would let the text pass see the binary pass's writes.
///
/// Declared column types are captured separately — see [`capture_types`].
async fn capture(client: &Client, p: &ParsedFixture) -> (Expected, Option<Expected>) {
    let isolate = !p.no_txn;
    let dual = !p.text_mode && p.params.is_empty() && !p.no_txn && p.expect_error.is_none();

    if !dual {
        let result =
            run_query_catching_errors(client, &p.setup, &p.test, p.text_mode, &p.params, isolate)
                .await;
        return (result, None);
    }

    if test_is_read_only(&p.test) {
        // One transaction, both formats — one setup.
        let (binary, text) = run_query_dual(client, &p.setup, &p.test, &p.params, isolate).await;
        (binary, Some(text))
    } else {
        // Mutating: each format runs in its own rolled-back txn (fresh setup).
        let binary =
            run_query_catching_errors(client, &p.setup, &p.test, false, &p.params, isolate).await;
        let text =
            run_query_catching_errors(client, &p.setup, &p.test, true, &p.params, isolate).await;
        (binary, Some(text))
    }
}

/// Whether a fixture has a text leg whose declared column types are worth
/// capturing: a plain success fixture (whose dual leg runs the text format) or a
/// `@text` one (whose compared result *is* the text format).
///
/// `@no-txn` is excluded because the text leg's types come from a *second*
/// connection replaying the setup ([`capture_types`]), which a fixture driving its own
/// transactions cannot be assumed to survive. The extended leg has no such exclusion —
/// see [`has_extended_leg`].
const fn has_text_leg(p: &ParsedFixture) -> bool {
    p.expect_error.is_none()
        && !p.no_txn
        && p.no_declared_types.is_none()
        && (p.text_mode || p.params.is_empty())
}

/// Whether a fixture has an **extended**-protocol leg whose declared column types are
/// worth comparing: any success fixture whose `@test` actually runs over the extended
/// protocol.
///
/// Two exclusions, and only two:
///
/// - `@text` — the fixture's compared result *is* the simple-query one, so no
///   statement is ever prepared and there is no `Describe` reply to read.
/// - `@expect-error` — pglet failing the query is the assertion. There is no agreed
///   `RowDescription`, and the deliberately-pglet-stricter half (Postgres accepts,
///   pglet rejects) would read as "the read failed on one side", which is a red.
///
/// Everything else is in, **including `@param` and `@no-txn`** — neither of which the
/// text leg can reach. The types ride along on the fixture's own binary run (no second
/// connection to replay setup on, so `@no-txn` is harmless) and a bound `$N` query is
/// prepared exactly as the spine prepares it.
const fn has_extended_leg(p: &ParsedFixture) -> bool {
    p.expect_error.is_none() && !p.text_mode && p.no_declared_types.is_none()
}

/// Capture the declared column types of a fixture's `@test` over the simple query
/// protocol, on the probe connection. Values are **not** read off this pass — see
/// [`run_text_types`] for why.
async fn capture_types(probe: &mut SimpleProbe, p: &ParsedFixture) -> Expected {
    run_text_types(probe, &p.setup, &p.test, !p.no_txn).await
}

// =============================================================================
// Update Mode
// =============================================================================

async fn run_update_mode(fixtures_dir: &Path, fixtures: &[PathBuf]) -> (usize, usize) {
    // Refused rather than warned about: update mode *writes* the committed
    // snapshots, and the session-identity fixtures would take the local oracle's
    // user and database spelling with them — a corpus every other contributor
    // then fails. The read-only path only warns.
    if !oracle_identity_is_default(&POSTGRES_CONFIG) {
        eprintln!(
            "[fixtures] refusing --update against user `{}` on database `{}`: the recorded \
             expectations hold the oracle's own session identity, so regenerating here would \
             commit this machine's spelling. Unset PGLET_PG_URL (or point it at \
             `postgres`/`postgres`) to regenerate.",
            POSTGRES_CONFIG.user, POSTGRES_CONFIG.database
        );
        std::process::exit(1);
    }
    // Only need Postgres for update mode
    if let Err(e) = ensure_postgres().await {
        eprintln!("[fixtures] {e}");
        std::process::exit(1);
    }
    // Same clean slate as test mode, so a regenerated expected reflects a fixture
    // run from an empty schema (not one polluted by leftover tables).
    if let Err(e) = reset_postgres_schema().await {
        eprintln!("[fixtures] {e}");
        std::process::exit(1);
    }

    println!("=== Update Mode ===\n");
    println!("Updating {} fixture(s)...\n", fixtures.len());

    let mut updated = 0;
    let mut errors = 0;

    for fixture_path in fixtures {
        let rel_path = fixture_path
            .strip_prefix(fixtures_dir)
            .unwrap_or(fixture_path);
        let name = rel_path.to_string_lossy();

        // Read and parse fixture
        let content = match fs::read_to_string(fixture_path) {
            Ok(c) => c,
            Err(e) => {
                println!("\x1b[31m✗\x1b[0m {name}: read error: {e}");
                errors += 1;
                continue;
            }
        };

        let parsed: ParsedFixture = match parse_fixture(&content, &name) {
            Ok(p) => p,
            Err(e) => {
                println!("\x1b[31m✗\x1b[0m {e}");
                errors += 1;
                continue;
            }
        };

        // Run against Postgres
        let pg_client = match connect(&POSTGRES_CONFIG).await {
            Ok(c) => c,
            Err(e) => {
                println!("\x1b[31m✗\x1b[0m {name}: Postgres connection failed: {e}");
                errors += 1;
                continue;
            }
        };

        let pg_result = run_query_catching_errors(
            &pg_client,
            &parsed.setup,
            &parsed.test,
            parsed.text_mode,
            &parsed.params,
            !parsed.no_txn,
        )
        .await;

        // Write expected file
        let exp_path = expected_path(fixture_path);
        let json = match serde_json::to_string_pretty(&pg_result) {
            Ok(j) => j,
            Err(e) => {
                println!("\x1b[31m✗\x1b[0m {name}: JSON serialize error: {e}");
                errors += 1;
                continue;
            }
        };

        if let Err(e) = fs::write(&exp_path, format!("{json}\n")) {
            println!("\x1b[31m✗\x1b[0m {name}: write error: {e}");
            errors += 1;
            continue;
        }

        println!("\x1b[32m✓\x1b[0m {name}");
        updated += 1;
    }

    (updated, errors)
}

// =============================================================================
// Profile lock
// =============================================================================

/// `Debug` renderings of the plan nodes that exist only under `index_acceleration`.
///
/// A fixture whose `@test` plans to one of these takes a *different plan* under the
/// stripped (`--no-default-features`) build, which emits `Filter(Scan)` instead — so
/// it is exactly the kind of fixture the stripped-profile lock must re-run. The
/// trailing `{` anchors the match to a struct-variant header, so a string literal in
/// the query can't pass for one.
#[cfg(feature = "index_acceleration")]
const ACCEL_PLAN_NODES: [&str; 3] = ["IndexScan {", "IndexRangeScan {", "GinScan {"];

/// Whether this fixture's `@test` plans to an index-accelerated node.
///
/// Planned in-process (this crate is built with acceleration on, whatever profile the
/// *spawned server* uses), by replaying the fixture's setup DDL into a scratch
/// in-memory `Database` and planning the test query against the resulting schema. A
/// setup statement that fails is tolerated — the schema it did build is usually
/// enough to plan against, and a false negative here is caught as a missing tag only
/// if the plan really is accelerated.
#[cfg(feature = "index_acceleration")]
fn test_plans_accelerated(parsed: &ParsedFixture) -> bool {
    let mut db = pglet::Database::with_btree();
    for statement in &parsed.setup {
        if !statement.trim().is_empty() {
            let _ = db.execute(statement);
        }
    }
    let Ok(plans) = pglet_sql::parser::parse(&parsed.test, db.schema()) else {
        // An `@expect-error` fixture that fails at plan time reaches no index
        // selection of its own; the deliberate decline-locks carry the tag by hand.
        return false;
    };
    let rendered = format!("{plans:?}");
    ACCEL_PLAN_NODES.iter().any(|node| rendered.contains(node))
}

/// Whether the fixture's `@setup` builds any index — an explicit `CREATE INDEX`, or
/// the implicit one behind a `UNIQUE` / `PRIMARY KEY` constraint.
///
/// A deliberately crude uppercase substring scan, not a parse: it over-reports (the
/// word inside a string literal counts) and that is the intended direction, since a
/// false positive costs one directive while a false negative is the silent omission
/// [`check_expect_error_index_tags`] exists to convert into a decision.
fn setup_creates_an_index(parsed: &ParsedFixture) -> bool {
    parsed.setup.iter().any(|statement| {
        let upper = statement.to_ascii_uppercase();
        upper.contains("CREATE INDEX") || upper.contains("UNIQUE") || upper.contains("PRIMARY KEY")
    })
}

/// Every fixture this binary owns, parsed once for the completeness checks.
struct FixtureSurvey {
    /// `(fixtures-dir-relative name, parsed fixture)`, in discovery order.
    parsed: Vec<(String, ParsedFixture)>,
    /// Rendered parse errors for the fixtures that are malformed rather than foreign.
    malformed: Vec<String>,
}

/// Parse every fixture for the completeness checks below.
fn survey_fixtures(fixtures_dir: &Path, fixtures: &[PathBuf]) -> FixtureSurvey {
    let mut survey = FixtureSurvey {
        parsed: Vec::new(),
        malformed: Vec::new(),
    };
    for fixture_path in fixtures {
        let name = fixture_path
            .strip_prefix(fixtures_dir)
            .unwrap_or(fixture_path)
            .to_string_lossy()
            .into_owned();
        let Ok(content) = fs::read_to_string(fixture_path) else {
            continue;
        };
        match parse_fixture(&content, &name) {
            Ok(parsed) => survey.parsed.push((name, parsed)),
            // Not ours to parse (the `copy/` family) — no plan to derive.
            Err(FixtureParseError::ForeignFamily(_)) => {}
            // A malformed fixture can't be classified, so it would slip past the
            // tag checks unnoticed — the very omission this guard exists to catch.
            Err(FixtureParseError::Invalid(e)) => survey.malformed.push(e),
        }
    }
    survey
}

/// Verify no fixture that plans to an accelerated node is missing `-- @profile-lock`.
///
/// This is what keeps the lock set from rotting. The tag is the selector, so a new
/// fixture that quietly picks up an index probe would otherwise sit outside the lock
/// run forever, and nobody would notice — the same silent-omission failure the lock
/// exists to prevent. Over-tagging is fine (the deliberate decline-locks reach index
/// selection and must *decline* identically, which produces no accelerated node);
/// under-tagging fails the run.
#[cfg(feature = "index_acceleration")]
fn check_accelerated_plan_tags(survey: &FixtureSurvey) -> bool {
    let untagged: Vec<&str> = survey
        .parsed
        .iter()
        .filter(|(_, parsed)| !parsed.profile_lock && test_plans_accelerated(parsed))
        .map(|(name, _)| name.as_str())
        .collect();
    if untagged.is_empty() {
        return true;
    }
    println!("\n\x1b[31m✗ profile-lock tag set is incomplete\x1b[0m");
    println!(
        "  {} fixture(s) plan to an index-accelerated node but lack `-- @profile-lock`.",
        untagged.len()
    );
    println!("  Add the directive to each, then re-run:");
    for name in &untagged {
        println!("    {name}");
    }
    false
}

#[cfg(not(feature = "index_acceleration"))]
fn check_accelerated_plan_tags(_survey: &FixtureSurvey) -> bool {
    // The derivation needs the accelerated planner to know which fixtures take an
    // index path at all. Built without it, the check can only pass vacuously — so
    // say so rather than report a green it did not earn.
    println!(
        "[fixtures] \x1b[33mskipping the accelerated-plan tag completeness check\x1b[0m: \
         this harness was built --no-default-features, so it cannot derive the accelerated plans"
    );
    true
}

/// Verify every `@expect-error` fixture whose `@setup` builds an index carries either
/// `-- @profile-lock` or a justified `-- @no-profile-lock:` opt-out.
///
/// [`check_accelerated_plan_tags`] is structurally blind to this class. Index
/// selection runs *inside* the planner, so a query that errors during it — a probe
/// type-mismatch, an operand gate — never yields a plan to inspect and is classified
/// "not accelerated". Yet that is exactly the shape where the profiles can disagree:
/// the accelerated build raises from the probe gate (green against Postgres in the
/// main run) while the stripped build's `Filter(Scan)` may return rows, or a
/// different SQLSTATE, and never runs at all.
///
/// The rule is a heuristic, not a proof — an index in the setup is a proxy for "an
/// index could be involved in the error". Its whole job is to turn a silent omission
/// into a forced decision: tag the fixture, or say in writing why it can't diverge.
fn check_expect_error_index_tags(survey: &FixtureSurvey) -> bool {
    let undecided: Vec<&str> = survey
        .parsed
        .iter()
        .filter(|(_, parsed)| {
            parsed.expect_error.is_some()
                && !parsed.profile_lock
                && parsed.no_profile_lock.is_none()
                && setup_creates_an_index(parsed)
        })
        .map(|(name, _)| name.as_str())
        .collect();
    if undecided.is_empty() {
        return true;
    }
    println!("\n\x1b[31m✗ profile-lock decision missing on `@expect-error` fixtures\x1b[0m");
    println!(
        "  {} `@expect-error` fixture(s) build an index in `@setup` but say nothing about \
         the build profiles.",
        undecided.len()
    );
    println!(
        "  An error raised during index selection is invisible to the plan-derived check, \
         so decide explicitly:"
    );
    println!("    `-- @profile-lock`                  — run it on the stripped profile too");
    println!("    `-- @no-profile-lock: <why>`        — it cannot diverge, and here is why");
    for name in &undecided {
        println!("    {name}");
    }
    false
}

/// Run every profile-lock completeness check over `fixtures`.
fn check_profile_lock_tags(fixtures_dir: &Path, fixtures: &[PathBuf]) -> bool {
    let survey = survey_fixtures(fixtures_dir, fixtures);
    if !survey.malformed.is_empty() {
        println!("\n\x1b[31m✗ malformed fixture(s)\x1b[0m");
        for e in &survey.malformed {
            println!("    {e}");
        }
    }
    // Every check runs, so one invocation reports the whole picture.
    let plans_ok = check_accelerated_plan_tags(&survey);
    let expect_error_ok = check_expect_error_index_tags(&survey);
    survey.malformed.is_empty() && plans_ok && expect_error_ok
}

// =============================================================================
// Main
// =============================================================================

/// Compare what pglet sent in the structured `ErrorResponse` fields — and, when the
/// fixture opts in with `@expect-message`, the primary message — against **live**
/// Postgres.
///
/// The comparison target is `pg_live`, not the recorded `.expected.json`: these fields
/// are never serialized (see `QueryError::fields`), so the recording is unchanged by
/// this whole capability and no fixture needed regenerating to gain it.
///
/// The scoring rule is asymmetric and that is the design: a field pglet **omits** is
/// agreement, because omitting what it cannot fill truthfully is the declared posture;
/// a field pglet sends with a value 18.6 does not send, or sends differently, is a
/// failure. `@no-error-fields: <why>` waives it for a justified divergence.
fn check_error_fields(
    parsed: &ParsedFixture,
    pg_live: &Expected,
    pglet_live: &Expected,
    pglet_err: &QueryError,
    pg_err: &QueryError,
) -> Result<(), String> {
    if parsed.expect_message && pglet_err.message != pg_err.message {
        return Err(format!(
            "message mismatch (@expect-message): Postgres {:?}, pglet {:?}",
            pg_err.message, pglet_err.message
        ));
    }
    if parsed.no_error_fields.is_some() {
        return Ok(());
    }
    // `pg_live` is a Postgres error here (the caller matched on the recorded shape,
    // which step 1 already proved matches live Postgres); `pglet_live` is pglet's.
    let diffs = pg_live
        .error_fields()
        .divergences(&pglet_live.error_fields());
    if diffs.is_empty() {
        Ok(())
    } else {
        Err(format!("error field mismatch: {}", diffs.join("; ")))
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let update_mode = args.iter().any(|a| a == "--update");
    let verbose = args.iter().any(|a| a == "-v" || a == "--verbose");
    let profile_lock_mode = args.iter().any(|a| a == "--profile-lock");
    let filter = args.iter().skip(1).find(|a| !a.starts_with('-'));

    // Find fixtures directory
    let fixtures_dir = PathBuf::from("tests/fixtures");
    if !fixtures_dir.exists() {
        eprintln!("[fixtures] Directory not found: tests/fixtures");
        std::process::exit(1);
    }

    // Discover fixtures
    let mut fixtures = discover_fixtures(&fixtures_dir);

    // Profile-lock mode: check the tag set is complete over *every* fixture (before
    // any filter narrows it), then run only the tagged ones. Under
    // `PGLET_NO_INDEX_ACCEL=1` the spawned server is the stripped build, so this is
    // the stripped profile's differential run over the fixtures whose plan differs.
    if profile_lock_mode {
        if !check_profile_lock_tags(&fixtures_dir, &fixtures) {
            std::process::exit(1);
        }
        if !stripped_profile_requested() {
            println!(
                "[fixtures] \x1b[33mnote\x1b[0m: PGLET_NO_INDEX_ACCEL is unset, so this runs the \
                 *accelerated* server — the lock's point is the stripped one"
            );
        }
        fixtures.retain(|p| {
            fs::read_to_string(p)
                .ok()
                .and_then(|c| parse_fixture(&c, &p.to_string_lossy()).ok())
                .is_some_and(|parsed| parsed.profile_lock)
        });
    }

    // Apply filter
    if let Some(pattern) = filter {
        fixtures.retain(|p| p.to_string_lossy().contains(pattern.as_str()));
    }

    if fixtures.is_empty() {
        println!("No fixtures found");
        return;
    }

    if update_mode {
        let (updated, errors) = run_update_mode(&fixtures_dir, &fixtures).await;
        println!("\n=== Results ===");
        println!("Updated: {updated}");
        println!("Errors:  {errors}");

        if errors > 0 {
            std::process::exit(1);
        }
    } else {
        let (passed, failed, skipped) = run_test_mode(&fixtures_dir, &fixtures, verbose).await;
        println!("\n=== Results ===");
        println!("Passed:  {passed}");
        println!("Failed:  {failed}");
        println!("Skipped: {skipped}");

        if failed > 0 {
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DeclaredTypeCoverage, has_extended_leg, has_text_leg};
    use pglet_tests::{DeclaredTypes, ParsedFixture, parse_fixture};

    /// A fixture carrying `directives` above a plain `SELECT` `@test`.
    fn fixture(directives: &str) -> ParsedFixture {
        parse_fixture(
            &format!("{directives}\n-- @test\nSELECT 1 AS a;\n"),
            "eligibility.sql",
        )
        .expect("the test fixtures here are well-formed")
    }

    /// The two legs read *different walkers*, so they must not share one corpus: a
    /// `@param` or `@no-txn` fixture the text leg cannot reach is exactly the kind the
    /// extended leg must still assert on, and a `@text` one is the reverse.
    #[test]
    fn the_two_legs_cover_different_fixtures() {
        // The plain case: both legs.
        let plain = fixture("");
        assert!(has_text_leg(&plain));
        assert!(has_extended_leg(&plain));

        // `@text` runs no extended protocol at all — there is no prepared statement
        // to describe.
        let text = fixture("-- @text");
        assert!(has_text_leg(&text));
        assert!(!has_extended_leg(&text));

        // `@param` binds over the extended protocol, which the simple query has no
        // way to do — extended-only, and a coverage *gain* over the text leg.
        let param = fixture("-- @param $1 int4 1");
        assert!(!has_text_leg(&param));
        assert!(has_extended_leg(&param));

        // `@no-txn` breaks the text leg's second connection replaying the setup; the
        // extended leg rides the fixture's own run and is unaffected.
        let no_txn = fixture("-- @no-txn");
        assert!(!has_text_leg(&no_txn));
        assert!(has_extended_leg(&no_txn));

        // `@expect-error`: pglet failing is the assertion, so neither leg applies.
        let expect_error = fixture("-- @expect-error: unsupported");
        assert!(!has_text_leg(&expect_error));
        assert!(!has_extended_leg(&expect_error));
    }

    /// One opt-out covers **both** legs. A per-leg waiver would let a fixture pass the
    /// executor walker while the planner walker went unchecked — which is precisely
    /// the pglet-internal split the extended leg exists to expose, so it must never be
    /// silenceable on one side alone.
    #[test]
    fn the_declared_types_opt_out_covers_both_legs() {
        let opted_out = fixture("-- @no-declared-types: pglet models no `name` type");
        assert!(!has_text_leg(&opted_out));
        assert!(!has_extended_leg(&opted_out));
    }

    /// The coverage tally must account for **every** eligible fixture, including the
    /// red ones. Folding a mismatch into no bucket is how a printed total quietly
    /// under-reports exactly when the number matters most.
    #[test]
    fn coverage_accounts_for_every_eligible_fixture() {
        let mut tally = DeclaredTypeCoverage {
            eligible: 5,
            ..DeclaredTypeCoverage::default()
        };
        assert!(tally.record(DeclaredTypes::Match).is_none());
        assert!(tally.record(DeclaredTypes::NoColumns).is_none());
        assert!(
            tally
                .record(DeclaredTypes::BothErrored("42704".into()))
                .is_none()
        );
        // The two red outcomes both hand back a line to print *and* land in a bucket.
        assert!(tally.record(DeclaredTypes::Mismatch("a".into())).is_some());
        assert!(
            tally
                .record(DeclaredTypes::ProbeFailed("b".into()))
                .is_some()
        );
        assert_eq!(tally.asserted, 1);
        assert_eq!(tally.mismatched, 1);
        assert_eq!(tally.probe_failed, 1);
        assert_eq!(
            tally.asserted
                + tally.no_columns
                + tally.both_errored
                + tally.mismatched
                + tally.probe_failed,
            tally.eligible,
            "every eligible fixture lands in exactly one bucket"
        );
        // `NotEligible` is the caller's to supply and belongs to no bucket here — the
        // count is taken at the eligibility decision, not at the outcome.
        assert!(tally.record(DeclaredTypes::NotEligible).is_none());
        assert_eq!(tally.asserted, 1);
    }
}
