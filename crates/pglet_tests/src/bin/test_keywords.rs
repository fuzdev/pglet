//! Keyword-as-identifier conformance sweep.
//!
//! For every keyword pglet's lexer recognizes (`ALL_KEYWORDS`), assert pglet
//! accepts or rejects it in each Postgres identifier position **exactly** as a
//! live Postgres does:
//!
//! - bare column (`SELECT kw …`)  — Postgres `ColId`: UNRESERVED ∪ COL_NAME only;
//! - qualified attribute (`s.kw`) — Postgres `ColLabel`: every keyword;
//! - `AS` label (`… AS kw`)       — Postgres `ColLabel`: every keyword;
//! - table name (`… FROM kw`)     — Postgres `ColId`: UNRESERVED ∪ COL_NAME only;
//! - function name (`kw(1)`)      — Postgres `func_name`, which admits TYPE_FUNC_NAME;
//!   probed for that class only (see `FUNCTION_NAME_CLASSES`).
//!
//! This is the executable net behind the `is_reserved` derivation: a future
//! keyword added with the wrong `PgClass`, or a parser position that drifts from
//! Postgres, fails here against ground truth rather than against a hand-written
//! expectation. Self-contained — spawns a pglet server, needs Postgres on :5432.
//!
//!   cargo run -p `pglet_tests` --bin `test_keywords`

// The query futures hold no params, but mirror the other test bins' allow for a
// local single-threaded tokio runtime.
#![allow(clippy::future_not_send)]

use pglet_sql::lexer::{ALL_KEYWORDS, PgClass};
use pglet_tests::{POSTGRES_CONFIG, connect, ensure_postgres, pglet_config, spawn_test_server};
use tokio_postgres::Client;

/// Keywords whose **bare-column** probe (`SELECT kw FROM …`) tests something
/// other than `ColId`-ness, so it can't compare against Postgres cleanly. These
/// are *correctly reserved* in pglet; the divergence is a separate grammar
/// feature, not keyword classification:
///
/// - `all` — `SELECT ALL …` is Postgres's set-quantifier, and `SELECT [ALL] FROM
///   t` (an empty target list) is a Postgres grammar pglet doesn't implement, so
///   `SELECT all FROM …` parses the quantifier in PG but fails in pglet. The
///   qualified-attr and `AS`-label probes (pure `ColLabel`) still run for `all`.
const BARE_COLUMN_SKIP: &[&str] = &["all"];

/// The keyword classes the function-name probe runs for. `SELECT kw(1)` tests the
/// `func_name` position only for a TYPE_FUNC_NAME word (`left(…)`, `join(…)`), every one
/// of which is a call there, answered or `42883`. For the other classes the spelling
/// reaches other grammar first: a reserved word is a clause (`SELECT where(1)` is an
/// empty target list and a `WHERE` on 18.6), and two COL_NAME words, `values` and
/// `between`, are refused by 18.6's grammar and read as calls here — a measured
/// divergence of the `ColId` primary path, not of this class.
const FUNCTION_NAME_CLASSES: &[PgClass] = &[PgClass::TypeFuncName];

/// Whether `client` accepts `sql`. Each probe is its own autocommit statement, so
/// a syntax error is a clean `Err` that leaves the connection usable for the next.
async fn accepts(client: &Client, sql: &str) -> bool {
    client.simple_query(sql).await.is_ok()
}

/// Whether `sql` **parsed the keyword as a name**, for a probe that can't fully
/// resolve (the table-name probe references a deliberately-absent table). A `42601`
/// syntax error means the keyword was rejected in that position; any other outcome
/// — success, or a non-syntax error like `42P01` "table does not exist" — means it
/// parsed as a name. Both Postgres and pglet use `42601`/`42P01` here.
async fn parses_as_name(client: &Client, sql: &str) -> bool {
    match client.simple_query(sql).await {
        Ok(_) => true,
        Err(e) => e.code() != Some(&tokio_postgres::error::SqlState::SYNTAX_ERROR),
    }
}

/// A probe: a position label, the (Postgres-and-pglet-identical) SQL, and whether
/// to judge it by parse-acceptance only (`parse_only`) rather than full success.
struct Probe {
    label: &'static str,
    sql: String,
    parse_only: bool,
}

/// The identifier positions probed per keyword. The derived table labels its
/// column with the *quoted* keyword (always legal), so a `ColId` keyword resolves
/// cleanly in the bare-column probe rather than erroring "column does not exist".
/// The table-name probe references an absent table, and the function-name probe names a
/// function that mostly does not exist, so both are judged by parse only.
fn probes(kw: &str) -> [Probe; 5] {
    let p = |label, sql, parse_only| Probe {
        label,
        sql,
        parse_only,
    };
    [
        p(
            "bare-column (ColId)",
            format!("SELECT {kw} FROM (SELECT 1 AS \"{kw}\") s"),
            false,
        ),
        p(
            "qualified-attr (ColLabel)",
            format!("SELECT s.{kw} FROM (SELECT 1 AS \"{kw}\") s"),
            false,
        ),
        p("AS-label (ColLabel)", format!("SELECT 1 AS {kw}"), false),
        p("table-name (ColId)", format!("SELECT 1 FROM {kw}"), true),
        p("function-name (func_name)", format!("SELECT {kw}(1)"), true),
    ]
}

#[tokio::main]
async fn main() {
    if let Err(e) = ensure_postgres().await {
        eprintln!("[keywords] {e}");
        std::process::exit(1);
    }
    let server = match spawn_test_server().await {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("[keywords] failed to start pglet server: {e}");
            std::process::exit(1);
        }
    };
    let pglet_cfg = pglet_config(server.port);
    let pg = match connect(&POSTGRES_CONFIG).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[keywords] Postgres connection failed: {e}");
            std::process::exit(1);
        }
    };
    let pglet = match connect(&pglet_cfg).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[keywords] pglet connection failed: {e}");
            std::process::exit(1);
        }
    };

    let mut mismatches = Vec::new();
    let mut checked = 0usize;
    let mut skipped = 0usize;
    for &kw in ALL_KEYWORDS {
        for probe in probes(kw.as_str()) {
            // Skip the bare-column probe for keywords whose post-`SELECT` grammar
            // role makes it a non-`ColId` test (documented in `BARE_COLUMN_SKIP`).
            if probe.label.starts_with("bare-column") && BARE_COLUMN_SKIP.contains(&kw.as_str()) {
                skipped += 1;
                continue;
            }
            if probe.label.starts_with("function-name")
                && !FUNCTION_NAME_CLASSES.contains(&kw.pg_class())
            {
                skipped += 1;
                continue;
            }
            let (pg_ok, pglet_ok) = if probe.parse_only {
                (
                    parses_as_name(&pg, &probe.sql).await,
                    parses_as_name(&pglet, &probe.sql).await,
                )
            } else {
                (
                    accepts(&pg, &probe.sql).await,
                    accepts(&pglet, &probe.sql).await,
                )
            };
            checked += 1;
            if pg_ok != pglet_ok {
                mismatches.push(format!(
                    "{:<12} [{:?}] {:<26} PG={pg_ok} pglet={pglet_ok}  ::  {}",
                    kw.as_str(),
                    kw.pg_class(),
                    probe.label,
                    probe.sql,
                ));
            }
        }
    }

    println!("=== keyword conformance sweep ===");
    println!(
        "checked {checked} probes across {} keywords (5 positions each; {skipped} probe(s) skipped)",
        ALL_KEYWORDS.len(),
    );
    if mismatches.is_empty() {
        println!("\x1b[32mOK\x1b[0m all keyword/position acceptances match Postgres");
    } else {
        eprintln!(
            "\x1b[31m{} MISMATCH(es) vs Postgres:\x1b[0m",
            mismatches.len()
        );
        for m in &mismatches {
            eprintln!("  {m}");
        }
        std::process::exit(1);
    }
}
