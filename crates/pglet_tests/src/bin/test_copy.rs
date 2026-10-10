//! Differential `COPY` tests: drive the COPY sub-protocol over the **wire** against
//! both a spawned `pglet_server` and the live Postgres oracle and compare.
//!
//! Self-contained: builds the server, spawns it on an open port (5434 region,
//! in-memory), connects both backends, runs every `tests/fixtures/copy/*.sql`, and
//! kills the server. Postgres must be reachable at whatever `PGLET_PG_URL` names
//! (`127.0.0.1:5432`, database `postgres`, unset); if it isn't the suite degrades
//! gracefully (prints a skip notice and exits 0).
//!
//! # Fixture format
//!
//! A small directive language (one directive per line):
//!
//! - `-- @setup` — DDL/seed run (verbatim, multi-statement) on **both** backends.
//! - `-- @copy-to` — a `COPY … TO STDOUT` statement; `copy_out` runs on both and the
//!   emitted bytes must be **byte-for-byte identical**. When the transfer is expected to
//!   fail, both backends must reject it, under the same rule the `FROM STDIN` direction
//!   takes.
//! - `-- @copy-from` — a `COPY … FROM STDIN` statement, followed by `-- @data`
//!   (literal COPY text lines, until EOF or a lone `\.`) `copy_in` into both backends,
//!   then `-- @verify` (a `SELECT` run on both whose rows must match). When the ingest
//!   is expected to fail, both backends must reject it under the same SQLSTATE — see
//!   [`assert_both_error`].
//! - `-- @data-hex` — the data block spelled as bytes instead: whitespace-separated
//!   two-digit hex, every line's bytes appended with no newline added. It is how a
//!   fixture sends a byte a `.sql` file should not hold (a NUL, which blinds `grep`) or
//!   cannot hold (an illegal UTF-8 byte, which `read_to_string` refuses). A fixture
//!   takes `@data` or `@data-hex`, never both.
//!
//! A directive line is recognized by `pglet_tests::directive_body`, the predicate the
//! main fixture parser uses, and one that names none of the directives above is a hard
//! parse error that fails the fixture. That is what refuses a misspelling of the
//! directive-line shape, such as `-- @data-hexx`: folded in as text, it would join the
//! `COPY` statement with its hex lines, a statement both engines refuse as a syntax
//! error, so the fixture would pass having sent no data. A misspelling `directive_body`
//! does not read as a directive line is not refused, and still folds into the `COPY`
//! statement as text with the hex lines after it, sending no data — `-- @ data-hex`,
//! `-- @@data-hex` and `-- @-data-hex` among them (`tests/README.md`). The check reads
//! every line, a `@data` block's included, so a COPY text line of the directive shape
//! cannot be written into a `@data` block; `@data-hex` spells it instead.
//!
//! A `@copy-to` fixture that expects a refusal still needs a `@verify`-less shape, and
//! gets one for free: the `To` arm compares the two outcomes rather than unwrapping
//! pglet's.
//!
//! # What a `@copy-from` fixture cannot express
//!
//! A `FROM STDIN` refusal that arrives **before** the `CopyInResponse` — a bad target
//! relation, or a column list naming an unknown or a repeated column — is outside this
//! suite. `tokio_postgres`'s `copy_in` drops its message sender on that error, which
//! makes the receiver emit a `CopyFail` + `Sync` the server is no longer in copy mode
//! to receive, and the connection task dies "unexpected message from server". It dies
//! against the **real Postgres oracle** the same way, so this is the client's limit and
//! not a divergence — but the next fixture then fails its schema reset, and an error
//! with no SQLSTATE fails the fixture that raised it too ([`assert_both_error`]). An
//! error fixture that fails while the data is being ingested, past the response, is
//! outside this class. Pin that class in process instead
//! (`crates/pglet/tests/copy.rs`), where the resulting table state is assertable too.

// This test-runner binary holds non-`Send` tokio-postgres futures across awaits on a
// single-threaded local runtime; they never cross a thread, so `future_not_send`
// doesn't apply. `.expect()` surfaces setup failures loudly — right for a harness.
#![allow(clippy::future_not_send, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use pglet_testing::server::{
    build_server, find_open_port, hard_kill, spawn_server, wait_for_ready,
};
use pglet_tests::{
    DYNAMIC_PORT_END, PGLET_TEST_PORT, POSTGRES_CONFIG, connect, directive_body, pglet_config,
    query_rows,
};
use tokio_postgres::Client;

// =============================================================================
// Fixture model + parser
// =============================================================================

/// A parsed COPY fixture: either a `TO STDOUT` byte-compare or a `FROM STDIN`
/// ingest-and-verify.
enum CopyFixture {
    /// `COPY … TO STDOUT`: compare the emitted bytes byte-for-byte.
    To { setup: String, copy_sql: String },
    /// `COPY … FROM STDIN`: ingest `data`, then compare the `verify` SELECT's rows
    /// (or, when ingest fails, assert both backends reject it).
    From {
        setup: String,
        copy_sql: String,
        data: Vec<u8>,
        verify_sql: String,
    },
}

/// The directive spellings [`parse_copy_fixture`] accepts, for the unknown-directive
/// message.
const KNOWN_COPY_DIRECTIVES: &str = "@setup @copy-to @copy-from @data @data-hex @verify";

#[derive(Clone, Copy)]
enum Section {
    None,
    Setup,
    CopyTo,
    CopyFrom,
    Data,
    DataHex,
    Verify,
}

/// Parse a COPY fixture file into a [`CopyFixture`].
fn parse_copy_fixture(content: &str) -> Result<CopyFixture, String> {
    let mut section = Section::None;
    let mut setup = String::new();
    let mut copy_to: Option<String> = None;
    let mut copy_from: Option<String> = None;
    let mut data: Vec<u8> = Vec::new();
    let mut data_spellings = (false, false);
    let mut verify = String::new();

    for line in content.lines() {
        if let Some(body) = directive_body(line.trim()) {
            section = match body {
                "setup" => Section::Setup,
                "copy-to" => {
                    copy_to.get_or_insert_with(String::new);
                    Section::CopyTo
                }
                "copy-from" => {
                    copy_from.get_or_insert_with(String::new);
                    Section::CopyFrom
                }
                "data" => {
                    data_spellings.0 = true;
                    Section::Data
                }
                "data-hex" => {
                    data_spellings.1 = true;
                    Section::DataHex
                }
                "verify" => Section::Verify,
                _ => {
                    return Err(format!(
                        "unknown directive on line {:?} (known: {KNOWN_COPY_DIRECTIVES})",
                        line.trim()
                    ));
                }
            };
            continue;
        }
        match section {
            Section::None => {} // leading comments / blanks
            Section::Setup => push_line(&mut setup, line),
            Section::CopyTo => push_line(copy_to.as_mut().expect("copy_to set"), line),
            Section::CopyFrom => push_line(copy_from.as_mut().expect("copy_from set"), line),
            Section::Verify => push_line(&mut verify, line),
            Section::Data => {
                // The data block is verbatim COPY text — leading whitespace/tabs are
                // significant, so the raw line is kept. A lone `\.` ends the data
                // (excluded; tokio-postgres signals end via the sink, not the marker).
                if line == "\\." {
                    section = Section::None;
                    continue;
                }
                data.extend_from_slice(line.as_bytes());
                data.push(b'\n');
            }
            Section::DataHex => {
                for token in line.split_whitespace() {
                    data.push(hex_byte(token).ok_or_else(|| {
                        format!("@data-hex: {token:?} is not a two-digit hex byte")
                    })?);
                }
            }
        }
    }
    if data_spellings == (true, true) {
        return Err("fixture has both @data and @data-hex".to_string());
    }

    match (copy_to, copy_from) {
        (Some(copy_sql), None) => Ok(CopyFixture::To {
            setup: setup.trim().to_string(),
            copy_sql: copy_sql.trim().to_string(),
        }),
        (None, Some(copy_sql)) => Ok(CopyFixture::From {
            setup: setup.trim().to_string(),
            copy_sql: copy_sql.trim().to_string(),
            data,
            verify_sql: verify.trim().to_string(),
        }),
        (Some(_), Some(_)) => Err("fixture has both @copy-to and @copy-from".to_string()),
        (None, None) => Err("fixture has neither @copy-to nor @copy-from".to_string()),
    }
}

/// One `@data-hex` token: exactly two hex digits.
fn hex_byte(token: &str) -> Option<u8> {
    if token.len() == 2 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        u8::from_str_radix(token, 16).ok()
    } else {
        None
    }
}

/// Append `line` (and a newline) to a section buffer.
fn push_line(buf: &mut String, line: &str) {
    buf.push_str(line);
    buf.push('\n');
}

// =============================================================================
// COPY drivers (tokio-postgres sink/stream)
// =============================================================================

/// Drive `COPY … TO STDOUT`, returning the concatenated emitted bytes.
async fn copy_out(client: &Client, sql: &str) -> Result<Vec<u8>, tokio_postgres::Error> {
    let stream = client.copy_out(sql).await?;
    tokio::pin!(stream);
    let mut buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk?);
    }
    Ok(buf)
}

/// Drive `COPY … FROM STDIN`, sending `data` and returning the ingested row count.
async fn copy_in(client: &Client, sql: &str, data: &[u8]) -> Result<u64, tokio_postgres::Error> {
    let sink = client.copy_in(sql).await?;
    tokio::pin!(sink);
    sink.send(Bytes::copy_from_slice(data)).await?;
    sink.finish().await
}

/// The SQLSTATE code of a tokio-postgres error, if it carries a DB error.
fn sqlstate(e: &tokio_postgres::Error) -> Option<String> {
    e.as_db_error().map(|d| d.code().code().to_string())
}

// =============================================================================
// Fixture execution
// =============================================================================

/// Reset a backend's `public` schema so each fixture starts from a clean slate (the
/// `DROP SCHEMA public CASCADE; CREATE SCHEMA public` idiom both backends support).
async fn reset_schema(client: &Client) -> Result<(), String> {
    client
        .simple_query("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .await
        .map_err(|e| format!("schema reset failed: {e}"))?;
    Ok(())
}

/// Run a fixture's setup (verbatim, multi-statement) on one backend.
async fn run_setup(client: &Client, setup: &str) -> Result<(), String> {
    if setup.is_empty() {
        return Ok(());
    }
    client
        .simple_query(setup)
        .await
        .map_err(|e| format!("setup failed: {e}"))?;
    Ok(())
}

/// Run one fixture against both backends and assert agreement.
async fn run_fixture(pg: &Client, pglet: &Client, fixture: &CopyFixture) -> Result<(), String> {
    reset_schema(pg).await?;
    reset_schema(pglet).await?;

    match fixture {
        CopyFixture::To { setup, copy_sql } => {
            run_setup(pg, setup).await?;
            run_setup(pglet, setup).await?;
            // A refusal is an outcome this direction has to be able to express, and it
            // could not: a `COPY … TO STDOUT` both engines reject — the duplicate
            // column list is one — reported as "pglet COPY TO failed" whatever
            // Postgres did. Both-error is compared exactly as the `FROM STDIN`
            // direction's is.
            let pg_out = copy_out(pg, copy_sql).await;
            let pglet_out = copy_out(pglet, copy_sql).await;
            match (pg_out, pglet_out) {
                (Ok(pg_bytes), Ok(pglet_bytes)) => {
                    if pg_bytes == pglet_bytes {
                        Ok(())
                    } else {
                        Err(format!(
                            "COPY TO bytes differ:\n    Postgres: {:?}\n    pglet:    {:?}",
                            String::from_utf8_lossy(&pg_bytes),
                            String::from_utf8_lossy(&pglet_bytes),
                        ))
                    }
                }
                (Err(pg_e), Err(pglet_e)) => assert_both_error(&pg_e, &pglet_e),
                (Ok(_), Err(e)) => Err(format!("Postgres accepted but pglet rejected: {e}")),
                (Err(e), Ok(_)) => Err(format!("Postgres rejected ({e}) but pglet accepted")),
            }
        }
        CopyFixture::From {
            setup,
            copy_sql,
            data,
            verify_sql,
        } => {
            run_setup(pg, setup).await?;
            run_setup(pglet, setup).await?;
            let pg_in = copy_in(pg, copy_sql, data).await;
            let pglet_in = copy_in(pglet, copy_sql, data).await;
            match (&pg_in, &pglet_in) {
                (Ok(_), Ok(_)) => compare_verify(pg, pglet, verify_sql).await,
                (Err(pg_e), Err(pglet_e)) => assert_both_error(pg_e, pglet_e),
                (Ok(_), Err(e)) => Err(format!("Postgres accepted but pglet rejected: {e}")),
                (Err(e), Ok(_)) => Err(format!("Postgres rejected ({e}) but pglet accepted")),
            }
        }
    }
}

/// Both backends ingested: compare the verify SELECT's rows.
async fn compare_verify(pg: &Client, pglet: &Client, verify_sql: &str) -> Result<(), String> {
    if verify_sql.is_empty() {
        return Err("@copy-from fixture is missing its @verify SELECT".to_string());
    }
    let pg_rows = query_rows(pg, verify_sql).await?;
    let pglet_rows = query_rows(pglet, verify_sql).await?;
    if pg_rows == pglet_rows {
        Ok(())
    } else {
        Err(format!(
            "verify rows differ:\n    Postgres: {pg_rows:?}\n    pglet:    {pglet_rows:?}"
        ))
    }
}

/// Both backends rejected a transfer (an error fixture): pass only when both carry the
/// **same** SQLSTATE.
///
/// An error with no SQLSTATE did not come from the server's error path — a dropped
/// connection or a client-side failure — so it is not a refusal, and it fails the
/// fixture whichever side it is on, both sides at once included. The main fixture
/// harness's declared-type comparison refuses to let a harness fault match itself for
/// the same reason (`CLIENT_FAULT_CODE`).
///
/// The comparison used to be on the two-character *class*, which cannot tell the two
/// answers a COPY column list gives apart: a repeated name is `42701` and an unknown one
/// `42703`, and the pair is the whole content of the interleaving assertion
/// (`COPY t (a, a, zz)` against `COPY t (zz, a, a)`). Class-level agreement would have
/// scored both fixtures green with either verdict. Exact is also the rule the main
/// fixture harness already states for an `@expect-error` both engines reject.
fn assert_both_error(
    pg_e: &tokio_postgres::Error,
    pglet_e: &tokio_postgres::Error,
) -> Result<(), String> {
    match (sqlstate(pg_e), sqlstate(pglet_e)) {
        (Some(a), Some(b)) if a == b => Ok(()),
        (Some(a), Some(b)) => Err(format!(
            "both errored but SQLSTATE differs: Postgres {a}, pglet {b}"
        )),
        (pg_code, pglet_code) => Err(format!(
            "an error carried no SQLSTATE, so there is no refusal to compare: Postgres \
             {pg_code:?} ({pg_e}), pglet {pglet_code:?} ({pglet_e})"
        )),
    }
}

// =============================================================================
// Discovery + main
// =============================================================================

fn discover_fixtures(dir: &Path) -> Vec<PathBuf> {
    let mut fixtures: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    fixtures.sort();
    fixtures
}

#[tokio::main]
async fn main() {
    let fixtures_dir = PathBuf::from("tests/fixtures/copy");
    if !fixtures_dir.exists() {
        eprintln!("[copy] no fixtures directory at tests/fixtures/copy");
        return;
    }
    let fixtures = discover_fixtures(&fixtures_dir);
    if fixtures.is_empty() {
        println!("[copy] no COPY fixtures found");
        return;
    }

    // Postgres is the differential ground truth; degrade gracefully if it's absent.
    if let Err(e) = pglet_tests::ensure_postgres().await {
        eprintln!("[copy] {e}");
        eprintln!("[copy] skipping COPY differential tests (Postgres unavailable)");
        return;
    }

    eprintln!("[copy] Building pglet_server...");
    if !build_server() {
        eprintln!("[copy] build failed");
        std::process::exit(1);
    }
    let port = find_open_port(PGLET_TEST_PORT..DYNAMIC_PORT_END);
    eprintln!("[copy] Spawning pglet_server on port {port} (in-memory)...");
    let server = spawn_server(port, None);
    if !wait_for_ready(port).await {
        eprintln!("[copy] server failed to start");
        hard_kill(server);
        std::process::exit(1);
    }

    let pglet_config = pglet_config(port);
    let pg = connect(&POSTGRES_CONFIG)
        .await
        .expect("connect to Postgres");
    let pglet = connect(&pglet_config).await.expect("connect to pglet");

    println!("\n=== COPY Differential Tests ===\n");
    println!("Running {} fixtures...\n", fixtures.len());

    let mut passed = 0;
    let mut failed = 0;
    for path in &fixtures {
        let name = path
            .strip_prefix(&fixtures_dir)
            .unwrap_or(path)
            .to_string_lossy();
        let outcome = match fs::read_to_string(path) {
            Ok(content) => match parse_copy_fixture(&content) {
                Ok(fixture) => run_fixture(&pg, &pglet, &fixture).await,
                Err(e) => Err(e),
            },
            Err(e) => Err(format!("read error: {e}")),
        };
        match outcome {
            Ok(()) => {
                println!("\x1b[32m✓\x1b[0m {name}");
                passed += 1;
            }
            Err(e) => {
                println!("\x1b[31m✗\x1b[0m {name}: {e}");
                failed += 1;
            }
        }
    }

    eprintln!("\n[copy] Shutting down server...");
    hard_kill(server);

    println!("\n=== Results ===");
    println!("Passed: {passed}");
    println!("Failed: {failed}");
    if failed > 0 {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `FROM STDIN` fixture whose data is spelled `@data-hex`, with `directive` on the
    /// line where the `@data-hex` directive stands.
    fn hex_fixture(directive: &str) -> String {
        format!(
            "-- @setup\nCREATE TABLE ct (a text);\n-- @copy-from\nCOPY ct FROM STDIN;\n\
             {directive}\n68 00 64 0a\n-- @verify\nSELECT a FROM ct;\n"
        )
    }

    // The discriminating assertion comes first: without the unknown-directive arm, each of
    // these parses clean with the line and its hex folded into the `COPY` statement and no
    // data. For `-- @data-hexx` that statement is `42601` on both engines (measured), so
    // the fixture goes green having asserted nothing.
    #[test]
    fn a_misspelled_directive_is_a_parse_error() {
        for typo in [
            "-- @data-hexx",
            "-- @datahex",
            "--@data_hex",
            "-- @data-hex 68",
            "-- @verfy",
            "-- @1copy-from",
        ] {
            let Err(message) = parse_copy_fixture(&hex_fixture(typo)) else {
                panic!("{typo:?} parsed as a fixture");
            };
            assert!(
                message.starts_with("unknown directive on line ") && message.contains(typo),
                "{typo:?}: {message}"
            );
        }
    }

    #[test]
    fn a_directive_in_a_data_block_is_read_as_a_directive() {
        let content = "-- @setup\nCREATE TABLE ct (a text);\n-- @copy-from\n\
                       COPY ct FROM STDIN;\n-- @data\nok\n-- @dta\n-- @verify\nSELECT a FROM ct;\n";
        assert!(parse_copy_fixture(content).is_err());
    }

    // The accept polarity: each known spelling, including the `--@` one the shared
    // predicate normalizes, still selects its section.
    #[test]
    fn the_known_directives_parse() {
        for spelling in ["-- @data-hex", "--@data-hex", "--   @data-hex"] {
            let Ok(CopyFixture::From {
                setup,
                copy_sql,
                data,
                verify_sql,
            }) = parse_copy_fixture(&hex_fixture(spelling))
            else {
                panic!("{spelling:?} did not parse as a FROM STDIN fixture");
            };
            assert_eq!(setup, "CREATE TABLE ct (a text);");
            assert_eq!(copy_sql, "COPY ct FROM STDIN;");
            assert_eq!(data, [0x68, 0x00, 0x64, 0x0a]);
            assert_eq!(verify_sql, "SELECT a FROM ct;");
        }
        let to = "-- @setup\nCREATE TABLE ct (a text);\n-- @copy-to\nCOPY ct TO STDOUT;\n";
        assert!(matches!(
            parse_copy_fixture(to),
            Ok(CopyFixture::To { ref copy_sql, .. }) if copy_sql == "COPY ct TO STDOUT;"
        ));
    }

    #[test]
    fn every_copy_fixture_in_the_corpus_parses() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/copy");
        let fixtures = discover_fixtures(&dir);
        assert!(!fixtures.is_empty(), "no fixtures under {}", dir.display());
        for path in &fixtures {
            let content = fs::read_to_string(path).expect("read a COPY fixture");
            if let Err(e) = parse_copy_fixture(&content) {
                panic!("{}: {e}", path.display());
            }
        }
    }
}
