//! Integration test utilities for pglet.
//!
//! Provides server lifecycle management and result comparison for fixture tests.
//!
//! # Protocol Notes
//!
//! A `@test`'s last statement runs on both wire formats: the extended protocol in binary
//! (compared against the recorded `.expected.json`, so the binary encoding matches Postgres
//! byte for byte) and the simple query protocol in text (compared live against Postgres).
//! Setup statements use the simple query protocol, for multi-statement support.

// The query futures hold a `Vec<Box<dyn ToSql + Sync>>` of bound params across awaits,
// which isn't `Send`. This is a test harness on a local tokio runtime — the futures are
// never sent across threads, so `future_not_send` doesn't apply.
#![allow(clippy::future_not_send)]

pub mod simple_probe;

pub use simple_probe::{ProbeError, ProbeOutput, SimpleProbe};

use pglet_testing::server::stripped_profile_requested;
use std::process::Stdio;
use std::time::Duration;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tokio_postgres::types::{Date, Timestamp, ToSql};
use tokio_postgres::{Client, NoTls, SimpleQueryMessage, SimpleQueryRow};

/// Default port for the Postgres oracle (a system service, unless
/// [`ORACLE_URL_VAR`] names another).
pub const POSTGRES_PORT: u16 = 5432;

/// Port for pglet test server (auto-spawned). Also start of dynamic port range.
pub const PGLET_TEST_PORT: u16 = 5434;

/// Port range for dynamic allocation (end, exclusive).
pub const DYNAMIC_PORT_END: u16 = 5500;

/// The environment variable that names the Postgres oracle: a libpq
/// `postgres://` URL, or a `key=value` conninfo string.
///
/// It is the repo's one oracle knob. Every harness that connects to the oracle
/// reads [`POSTGRES_CONFIG`], which is derived from this variable once per
/// process; nothing addresses the oracle by a second copy of the constant. Left
/// unset, [`POSTGRES_CONFIG`] is exactly the historical default —
/// `host=127.0.0.1 port=5432 user=postgres dbname=postgres`, no password.
///
/// It exists because the fixture, dump and copy harnesses issue `DROP SCHEMA
/// public CASCADE` against whatever database this names. A contributor points it
/// at a scratch database rather than at the one they keep things in.
pub const ORACLE_URL_VAR: &str = "PGLET_PG_URL";

/// Connection configuration.
///
/// The string fields are `&'static str` so [`pglet_config`] stays a `const fn`
/// and every caller keeps holding a plain `&DbConfig`; the env-derived oracle
/// config leaks its parsed strings once, at first use.
#[derive(Debug, Clone)]
pub struct DbConfig {
    pub host: &'static str,
    pub port: u16,
    pub user: &'static str,
    pub database: &'static str,
    /// The password to authenticate with, when the configuration carries one.
    ///
    /// Only an oracle config parsed from [`ORACLE_URL_VAR`] ever does: a pglet
    /// server takes any credentials, so [`pglet_config`] leaves it `None`.
    pub password: Option<&'static str>,
}

impl DbConfig {
    /// Connection string for tokio-postgres, and the positional conninfo the
    /// `psql` / `pg_dump` shell-outs pass.
    ///
    /// The `key=value` form rather than the URL the override may have been
    /// spelled as, because it is the one form all three consumers accept and
    /// [`SimpleProbe`] needs the fields split out regardless.
    ///
    /// Every string value is rendered through [`quote_conninfo_value`], so a value the
    /// parse accepted is the value each consumer receives: a password or a
    /// database name holding a space used to render as a bare `key=a b`, which
    /// libpq reads as a value plus a keyword and refuses — a well-formed URL
    /// diagnosed as an unreachable server, and a value holding ` dbname=other`
    /// silently retargeting the connection.
    #[must_use]
    pub fn connection_string(&self) -> String {
        let mut s = format!(
            "host={} port={} user={} dbname={}",
            quote_conninfo_value(self.host),
            self.port,
            quote_conninfo_value(self.user),
            quote_conninfo_value(self.database),
        );
        if let Some(password) = self.password {
            use std::fmt::Write as _;
            let _ = write!(s, " password={}", quote_conninfo_value(password));
        }
        s
    }
}

/// Render one conninfo value per libpq's `key = value` grammar.
///
/// The grammar is `PQconnectdb`'s (`src/interfaces/libpq/fe-connect.c`'s
/// `conninfo_parse`): an unquoted value ends at the first whitespace, a `'…'`
/// value ends at the first unescaped `'`, and `\` escapes the next character in
/// either. `tokio_postgres::Config`'s own `Parser` implements the same rule, so
/// one rendering serves both consumers.
///
/// Quoted when the value is empty or holds any `char::is_whitespace` character,
/// `'` or `\` — a wider test than libpq's ASCII `isspace`, so a value such as one
/// holding a no-break space is quoted where libpq would not have re-split it
/// (safe: a quoted value round-trips unchanged through both parsers). Every
/// other value renders bare, so the default target still renders as the
/// historical `host=127.0.0.1 port=5432 user=postgres dbname=postgres`.
#[must_use]
pub fn quote_conninfo_value(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c == '\'' || c == '\\')
    {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// Config for the Postgres oracle, read from [`ORACLE_URL_VAR`] once per process.
///
/// A malformed value panics on first use rather than falling back to the
/// default: a typo silently running the schema-dropping harnesses against
/// `127.0.0.1:5432` is the failure this knob exists to prevent.
pub static POSTGRES_CONFIG: std::sync::LazyLock<DbConfig> = std::sync::LazyLock::new(|| {
    let url = std::env::var(ORACLE_URL_VAR).ok();
    match oracle_config(url.as_deref()) {
        Ok(config) => config,
        // A panic, and deliberately not a fallback: every reader of this static
        // is a harness that would otherwise run `DROP SCHEMA public CASCADE`
        // against the default target the operator was steering away from.
        #[allow(clippy::panic, reason = "a misconfigured oracle must not run at all")]
        Err(message) => panic!("{message}"),
    }
});

/// The oracle config `url` names, or the historical default when it is `None`.
///
/// Parsing is `tokio_postgres::Config`'s, which accepts both the `postgres://`
/// URL and the `key=value` conninfo spellings — so the harness cannot disagree
/// with the client library about what the value meant. The fields are then split
/// back out because [`SimpleProbe`] speaks the startup message by hand, one
/// field at a time.
///
/// # Errors
///
/// A message naming [`ORACLE_URL_VAR`] when the value does not parse, names
/// something other than exactly one TCP host, or omits the user or the database
/// name. Nothing is defaulted per-field: a partially-specified override would
/// silently mix in the historical target this variable exists to move away from.
pub fn oracle_config(url: Option<&str>) -> Result<DbConfig, String> {
    let Some(url) = url else {
        return Ok(DbConfig {
            host: "127.0.0.1",
            port: POSTGRES_PORT,
            user: "postgres",
            database: "postgres",
            password: None,
        });
    };
    // Trimmed before it is parsed *and* before it is quoted back in a refusal,
    // because libpq trims a conninfo's surrounding whitespace and the URL form
    // does not: `…/scratch ` would otherwise resolve to the database `scratch`
    // for a shell-out and to `scratch ` for the harness's own report of what it
    // resolved, which is the report disagreeing with the connection.
    let url = url.trim();
    let parsed: tokio_postgres::Config = url
        .parse()
        .map_err(|e| format!("{ORACLE_URL_VAR}=`{url}` is not a valid connection string: {e}"))?;

    let host = match parsed.get_hosts() {
        [tokio_postgres::config::Host::Tcp(host)] => host.clone(),
        [] => return Err(format!("{ORACLE_URL_VAR}=`{url}` names no host")),
        [_] => {
            return Err(format!(
                "{ORACLE_URL_VAR}=`{url}` names a Unix socket; the harness speaks TCP only"
            ));
        }
        _ => {
            return Err(format!(
                "{ORACLE_URL_VAR}=`{url}` names several hosts; the harness connects to exactly one"
            ));
        }
    };
    let port = parsed.get_ports().first().copied().unwrap_or(POSTGRES_PORT);
    let user = parsed
        .get_user()
        .ok_or_else(|| format!("{ORACLE_URL_VAR}=`{url}` names no user"))?;
    let database = parsed
        .get_dbname()
        .ok_or_else(|| format!("{ORACLE_URL_VAR}=`{url}` names no database"))?;
    let password = parsed
        .get_password()
        .map(|bytes| {
            std::str::from_utf8(bytes)
                .map_err(|_| format!("{ORACLE_URL_VAR}=`{url}` has a non-UTF-8 password"))
        })
        .transpose()?;

    Ok(DbConfig {
        host: leak(&host),
        port,
        user: leak(user),
        database: leak(database),
        password: password.map(leak),
    })
}

/// Whether the oracle's *session identity* — its user and database names — is the
/// historical default.
///
/// A handful of fixtures record what `current_user` / `current_database()` answer,
/// and the answer is the oracle's own user and database spelling. So an override
/// that moves either name makes those fixtures report their recorded expectation
/// as stale, and regenerating the corpus under it would bake the local spelling
/// into the committed snapshots. Callers use this to say so instead of leaving a
/// reader to infer it from a diff.
#[must_use]
pub fn oracle_identity_is_default(config: &DbConfig) -> bool {
    config.user == "postgres" && config.database == "postgres"
}

/// Leak a parsed connection field so [`DbConfig`] keeps its `&'static str`
/// fields. Bounded by the number of configs a process derives — one, in every
/// harness.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_owned().into_boxed_str())
}

/// Point a `psql` / `pg_dump` invocation at `config`: its conninfo, positionally.
///
/// Positional because libpq ranks a conninfo argument above the `PG*`
/// environment — so this argument is what decides where the child connects, and
/// an inherited `PGHOST`/`PGDATABASE` cannot quietly redirect it. It carries the
/// password too when the config has one, which is why these call sites no longer
/// set `PGPASSWORD`: a literal in the environment was a second copy of a
/// credential the config already holds, and it was set even for the pglet-server
/// targets, which take any credentials at all.
pub fn apply_conninfo(command: &mut std::process::Command, config: &DbConfig) {
    command.arg(config.connection_string());
}

/// Connection config for a pglet test server on `port`.
///
/// The port is dynamic — [`spawn_test_server`] picks an open one per run rather than a
/// fixed slot, so callers build their config from the returned `ServerHandle::port`
/// instead of a hardcoded constant (a fixed port invited connecting to a *stale*
/// leftover server from an earlier run).
#[must_use]
pub const fn pglet_config(port: u16) -> DbConfig {
    DbConfig {
        host: "127.0.0.1",
        port,
        user: "postgres",
        database: "postgres",
        password: None,
    }
}

/// Handle to a spawned pglet server process.
#[derive(Debug)]
pub struct ServerHandle {
    process: Child,
    pub port: u16,
}

impl ServerHandle {
    /// Kill the server process.
    pub async fn kill(&mut self) {
        let _ = self.process.kill().await;
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        // Try to kill on drop (best effort, non-async)
        let _ = self.process.start_kill();
    }
}

/// Check if a Postgres-compatiblesque server is healthy.
///
/// # Errors
///
/// The underlying `tokio_postgres` error text, carried rather than collapsed to a
/// bool: "not available" is the right diagnosis for a server that is down and the
/// wrong one for every other reason a connection fails — a refused password, a
/// database that does not exist, a conninfo the client library will not accept.
/// Swallowing the text made each of those read as "start Postgres".
pub async fn check_health(config: &DbConfig) -> Result<(), String> {
    let conn_str = config.connection_string();
    match tokio_postgres::connect(&conn_str, NoTls).await {
        Ok((client, connection)) => {
            // Spawn connection task (required by tokio-postgres)
            tokio::spawn(async move {
                let _ = connection.await;
            });
            // Try a simple query
            client
                .simple_query("SELECT 1")
                .await
                .map(|_| ())
                .map_err(|e| error_chain(&e))
        }
        Err(e) => Err(error_chain(&e)),
    }
}

/// An error and its `source` chain, joined.
///
/// `tokio_postgres::Error`'s own `Display` is a category — "error connecting to
/// server", "db error" — and the sentence a reader needs (`Connection refused`,
/// `database "…" does not exist`) is one `source` down. Reporting only the
/// category is how a missing database read as a server that is down.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        use std::fmt::Write as _;
        let _ = write!(text, ": {cause}");
        source = cause.source();
    }
    text
}

/// Wait for a server to become healthy.
pub async fn wait_for_server(config: &DbConfig, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    let interval = Duration::from_millis(100);

    while start.elapsed() < timeout {
        if check_health(config).await.is_ok() {
            return true;
        }
        sleep(interval).await;
    }

    false
}

/// Find workspace root by looking for a `Cargo.toml` with a `[workspace]` table.
fn find_workspace_root() -> Result<std::path::PathBuf, String> {
    let mut dir = std::env::current_dir().map_err(|e| format!("Failed to get cwd: {e}"))?;

    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let content =
                std::fs::read_to_string(&cargo_toml).map_err(|e| format!("Read error: {e}"))?;
            if content.contains("[workspace]") {
                return Ok(dir);
            }
        }

        if !dir.pop() {
            return Err("Could not find workspace root".to_string());
        }
    }
}

/// Build the `pglet_server` binary the differential harnesses spawn, in the profile
/// [`stripped_profile_requested`] selects.
///
/// The profile check is **not optional**: without it `PGLET_NO_INDEX_ACCEL=1
/// deno task test:fixtures` builds and spawns the *accelerated* server, so the
/// stripped profile's whole differential run is a silent no-op — a green bar against
/// the wrong engine.
pub async fn build_server() -> Result<std::path::PathBuf, String> {
    let workspace_root = find_workspace_root()?;

    let mut cmd = Command::new("cargo");
    cmd.args(["build", "-p", "pglet_server"]);
    if stripped_profile_requested() {
        cmd.arg("--no-default-features");
    }
    let output = cmd
        .current_dir(&workspace_root)
        .output()
        .await
        .map_err(|e| format!("Failed to run cargo build: {e}"))?;

    if output.status.success() {
        Ok(workspace_root)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("cargo build failed: {stderr}"))
    }
}

/// Spawn a pglet server on the given port.
pub async fn spawn_server(port: u16) -> Result<ServerHandle, String> {
    // Build first
    let workspace_root = build_server().await?;

    // Spawn server
    let binary_path = workspace_root.join("target/debug/pglet_server");
    let process = Command::new(&binary_path)
        .args(["--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to spawn server: {e}"))?;

    let config = DbConfig {
        host: "127.0.0.1",
        port,
        user: "postgres",
        database: "postgres",
        password: None,
    };

    // Wait for server to be ready
    if !wait_for_server(&config, Duration::from_secs(10)).await {
        return Err(format!("Server failed to start on port {port} within 10s"));
    }

    Ok(ServerHandle { process, port })
}

/// Spawn a fresh pglet server for a test run and return its handle (kills the server
/// on drop).
///
/// Always spawns a new server — built from current source (`spawn_server` runs
/// `cargo build` first) on a dynamically-chosen open port — rather than reusing one
/// already listening. Reusing a leftover server is a footgun: it may be a *stale* binary
/// from before an engine change, or carry *leaked* in-memory catalog state from an
/// interrupted prior run, either of which silently skews results (a green bar against the
/// wrong engine, or a red bar from another run's residue). Picking a fresh open port each
/// run also sidesteps a stale process still holding the base port. Connect to the server
/// with `pglet_config(handle.port)`.
pub async fn spawn_test_server() -> Result<ServerHandle, String> {
    let port = pglet_testing::server::find_open_port(PGLET_TEST_PORT..DYNAMIC_PORT_END);
    spawn_server(port).await
}

/// Ensure Postgres is running.
pub async fn ensure_postgres() -> Result<(), String> {
    if let Err(reason) = check_health(&POSTGRES_CONFIG).await {
        // The address is reported rather than assumed: with `PGLET_PG_URL` set,
        // "start postgresql" is the wrong advice about the wrong server, and the
        // reader has no other way to see which target the harness resolved. The
        // client library's own text rides along for the same reason: the failure
        // is as often authentication or a missing database as a server that is
        // down, and those need a different fix.
        let target = &*POSTGRES_CONFIG;
        return Err(format!(
            "Postgres not reachable on {}:{} (database `{}`, user `{}`): {reason}\nFix: start it, \
             or point {ORACLE_URL_VAR} at one",
            target.host, target.port, target.database, target.user
        ));
    }
    Ok(())
}

/// Connect to a database and return the client.
pub async fn connect(config: &DbConfig) -> Result<Client, tokio_postgres::Error> {
    let conn_str = config.connection_string();
    let (client, connection) = tokio_postgres::connect(&conn_str, NoTls).await?;

    // Spawn connection task
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {e}");
        }
    });

    // Pin the session to UTC so `timestamptz` text output is deterministic and
    // comparable. pglet has no session time zone (it always renders UTC + `+00`);
    // real Postgres renders `timestamptz` in its session zone, so without this a
    // `@text` differential fixture would diff pglet's `+00` against the harness
    // host's local offset. pglet takes `SET TIME ZONE 'UTC'` as the zone it already has
    // (and refuses any other), so this is the contract made explicit: pglet ≡ Postgres
    // run at `TimeZone=UTC`.
    client.simple_query("SET TIME ZONE 'UTC'").await?;

    Ok(client)
}

/// Whether a client binary (`psql` / `pg_dump`) is on `PATH` (`<name> --version`
/// exits 0). The shared skip-guard for the differential harnesses that shell out to
/// the Postgres client tools.
#[must_use]
pub fn has_binary(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Run a `SELECT` over the simple-query (text) protocol and collect its rows.
///
/// Returns a grid of rendered cell text (`None` = SQL NULL) — the shared comparison
/// shape for the `test_copy` / `test_dump` / `test_pg_dump` differential harnesses.
///
/// # Errors
///
/// Returns the formatted query error (callers fold it into their own result).
pub async fn query_rows(client: &Client, sql: &str) -> Result<Vec<Vec<Option<String>>>, String> {
    // Naming the statement and unwrapping the backend's own message, because
    // `tokio_postgres::Error` displays as the bare `db error` and hides both behind its
    // source — so a harness failure here used to read `query failed: db error` with
    // neither the SQL nor the SQLSTATE, which is indistinguishable across every
    // statement a caller runs.
    let messages = client.simple_query(sql).await.map_err(|e| {
        e.as_db_error().map_or_else(
            || format!("query failed for `{sql}`: {e}"),
            |db| {
                format!(
                    "query failed for `{sql}`: {} {}",
                    db.code().code(),
                    db.message()
                )
            },
        )
    })?;
    let mut rows = Vec::new();
    for message in messages {
        if let SimpleQueryMessage::Row(row) = message {
            let cells = (0..row.columns().len())
                .map(|i| row.get(i).map(str::to_string))
                .collect();
            rows.push(cells);
        }
    }
    Ok(rows)
}

/// Query result for comparison.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueryResult {
    pub rows: Vec<serde_json::Value>,
    #[serde(rename = "rowCount")]
    pub row_count: usize,
}

/// Query error for comparison.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueryError {
    pub code: String,
    pub message: String,
    /// The **structured** `ErrorResponse` fields this server sent alongside the
    /// SQLSTATE — `D`/`H`/`P`/`s`/`t`/`c`/`n`.
    ///
    /// Never serialized, for the same reason `Expected::Success::column_types` isn't:
    /// it is a live pglet-vs-Postgres assertion, not part of the recorded shape. That
    /// choice is what keeps this widening from touching a single `.expected.json` —
    /// the 897 recorded errors are unchanged, and a divergence can't be frozen into
    /// one by an `--update` run.
    #[serde(skip)]
    pub fields: ErrorFields,
}

/// The structured `ErrorResponse` fields a server sent, by `pg`'s property names.
///
/// Read off tokio-postgres's `DbError` accessors, which cover every field either
/// engine sends. `None` means the field was **absent**, which for pglet is the honest
/// posture for anything it cannot fill truthfully — [`ErrorFields::divergences`]
/// scores an absence as agreement and a *different value* as a failure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ErrorFields {
    /// `D` — DETAIL.
    pub detail: Option<String>,
    /// `H` — HINT.
    pub hint: Option<String>,
    /// `P` — statement position, a 1-based codepoint index.
    pub position: Option<u32>,
    /// `s` — schema name.
    pub schema: Option<String>,
    /// `t` — table name.
    pub table: Option<String>,
    /// `c` — column name.
    pub column: Option<String>,
    /// `n` — constraint name.
    pub constraint: Option<String>,
}

impl ErrorFields {
    /// Every field where **pglet sent a value that differs from Postgres's**, rendered
    /// for the failure report.
    ///
    /// `self` is Postgres, `pglet` is pglet. The asymmetry is the whole rule:
    ///
    /// - pglet omits a field 18.6 sends → **not** a divergence. Omission is the
    ///   declared posture for a field pglet cannot fill truthfully, and scoring it red
    ///   would make the honest answer indistinguishable from a wrong one.
    /// - pglet sends a field 18.6 omits → a divergence. That is an *invention*, the
    ///   failure this comparison exists to catch.
    /// - both send it and the values differ → a divergence.
    #[must_use]
    pub fn divergences(&self, pglet: &Self) -> Vec<String> {
        let mut out = Vec::new();
        let mut check = |name: &str, pg: Option<&str>, mine: Option<&str>| {
            if let Some(mine) = mine
                && pg != Some(mine)
            {
                out.push(format!(
                    "{name}: Postgres {}, pglet {mine:?}",
                    pg.map_or_else(|| "(absent)".to_string(), |v| format!("{v:?}"))
                ));
            }
        };
        check("detail", self.detail.as_deref(), pglet.detail.as_deref());
        check("hint", self.hint.as_deref(), pglet.hint.as_deref());
        check(
            "position",
            self.position.map(|p| p.to_string()).as_deref(),
            pglet.position.map(|p| p.to_string()).as_deref(),
        );
        check("schema", self.schema.as_deref(), pglet.schema.as_deref());
        check("table", self.table.as_deref(), pglet.table.as_deref());
        check("column", self.column.as_deref(), pglet.column.as_deref());
        check(
            "constraint",
            self.constraint.as_deref(),
            pglet.constraint.as_deref(),
        );
        out
    }

    /// Read the structured fields off a tokio-postgres `DbError`.
    #[must_use]
    pub fn from_db_error(e: &tokio_postgres::error::DbError) -> Self {
        use tokio_postgres::error::ErrorPosition;
        Self {
            detail: e.detail().map(ToString::to_string),
            hint: e.hint().map(ToString::to_string),
            position: match e.position() {
                Some(ErrorPosition::Original(p)) => Some(*p),
                _ => None,
            },
            schema: e.schema().map(ToString::to_string),
            table: e.table().map(ToString::to_string),
            column: e.column().map(ToString::to_string),
            constraint: e.constraint().map(ToString::to_string),
        }
    }
}

/// One output column as a server **declared** it in a `RowDescription`: its name and
/// type OID.
///
/// Deliberately not named for either producer, because there are two and they are the
/// point. pglet computes its declared types through **different code** per protocol —
/// the simple query's from the executor's `QueryResult::shape` (`expr_type_oid`), the
/// extended protocol's from the planner's `Plan::result_shape` (`Expr::infer_type`)
/// via `Describe` — and a binary client decodes against the *second* one. A shape named
/// after one leg would invite the two to be compared as if they were one thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredColumn {
    pub name: String,
    pub type_oid: u32,
}

/// The declared column types `statement` reported in its extended-protocol `Describe`
/// reply — the planner walker's answer, and the one a binary client decodes against.
///
/// tokio-postgres keeps the prepared statement's `RowDescription` (unlike the simple
/// query's, which it discards — see [`SimpleProbe`]), so this leg needs no hand-rolled
/// protocol code at all: `Statement::columns` is built field-by-field from that reply
/// and `Type::oid` hands the raw OID back.
#[must_use]
pub fn statement_declared_columns(statement: &tokio_postgres::Statement) -> Vec<DeclaredColumn> {
    statement
        .columns()
        .iter()
        .map(|c| DeclaredColumn {
            name: c.name().to_string(),
            type_oid: c.type_().oid(),
        })
        .collect()
}

/// The outcome of comparing the declared column types of one of a fixture's legs —
/// see [`Expected::compare_declared_types`].
///
/// Four outcomes and not a `bool`/`Option` because the check has two distinct
/// non-assertions, and conflating them is a silent-green hole: nothing to compare
/// (a statement that describes no row) is a legitimate pass, while no comparison
/// having happened (the probe itself failed) is a harness defect that must be
/// reported. The failure mode is the same one [`CLIENT_FAULT_CODE`] closes on the
/// `@expect-error` arm — an error in the checking machinery scoring as the absence
/// of a divergence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclaredTypes {
    /// The fixture has no leg of this kind to read (`@expect-error`, a `@param`/
    /// `@no-txn` fixture on the text leg, a `@text` fixture on the extended leg, or a
    /// justified `@no-declared-types:` opt-out), so the check does not apply. Never
    /// produced by [`Expected::compare_declared_types`] — the caller supplies it, so
    /// that a fixture with nothing to read cannot be mistaken for one whose two sides
    /// agreed on nothing.
    NotEligible,
    /// Both sides declared the same column list. The assertion ran and held.
    Match,
    /// Both probes ran and neither described a row: the `@test`'s last statement
    /// emits no `RowDescription` (an `INSERT`/`UPDATE`/DDL without `RETURNING`).
    /// A structural non-assertion, so it needs no per-fixture opt-out.
    NoColumns,
    /// The `@test` errored on **both** engines with the same SQLSTATE — a fixture
    /// whose recorded expectation is itself an error. A failed statement describes
    /// no columns, so there is nothing to compare, and the error is already
    /// asserted by the value comparison. Distinct from [`Self::ProbeFailed`]: the
    /// two engines agreeing on a rejection is a result, not a broken probe.
    BothErrored(String),
    /// The two sides declared different columns — a differing count (**including**
    /// one side describing no row while the other does) or a per-column OID
    /// difference. The string renders the divergence for the report.
    Mismatch(String),
    /// The probe itself failed on at least one side, so no comparison happened.
    /// Not a pass: it is the absence of evidence, and the fixture's declared types
    /// are unchecked until whatever broke the probe is fixed.
    ProbeFailed(String),
}

/// Query output (either success or error).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum Expected {
    Success {
        rows: Vec<serde_json::Value>,
        #[serde(rename = "rowCount")]
        row_count: usize,
        /// The **declared** column types the server sent in its `RowDescription` for
        /// *this* result — the simple-query one via [`SimpleProbe`], the extended one
        /// off the prepared statement's `Describe` reply (see
        /// [`statement_declared_columns`]). Which leg filled it is the caller's to
        /// track; the two are never mixed in one `Expected`, and comparing across legs
        /// would compare two different walkers' answers as though they were one.
        ///
        /// Never serialized: it is a live pglet-vs-Postgres assertion, not part of
        /// the recorded shape, so `.expected.json` stays a pure value snapshot and
        /// a type divergence can't be frozen into one by an `--update` run.
        #[serde(skip)]
        column_types: Vec<DeclaredColumn>,
    },
    Error {
        error: QueryError,
    },
}

impl Expected {
    /// Create a success result with no captured declared types — for the callers that
    /// have none to record (a text-mode run, whose values come back through
    /// tokio-postgres's `SimpleQueryRow` with the `RowDescription` already discarded,
    /// or an empty placeholder).
    #[must_use]
    pub const fn success(rows: Vec<serde_json::Value>, row_count: usize) -> Self {
        Self::Success {
            rows,
            row_count,
            column_types: Vec::new(),
        }
    }

    /// Create a success result carrying the declared column types one leg's
    /// `RowDescription` reported.
    #[must_use]
    pub const fn success_typed(
        rows: Vec<serde_json::Value>,
        row_count: usize,
        column_types: Vec<DeclaredColumn>,
    ) -> Self {
        Self::Success {
            rows,
            row_count,
            column_types,
        }
    }

    /// The declared column types this result captured (empty where its leg has none).
    #[must_use]
    pub fn column_types(&self) -> &[DeclaredColumn] {
        match self {
            Self::Success { column_types, .. } => column_types,
            Self::Error { .. } => &[],
        }
    }

    /// Compare the column types `self` (Postgres) and `other` (pglet) **declared**
    /// for the same result. Leg-agnostic: the caller pairs two results from the *same*
    /// leg, and both legs get the identical count-then-per-column-OID comparison so
    /// they cannot drift into asserting different things.
    ///
    /// The values already compared equal by the time this runs, which is exactly
    /// why it is needed. On the **text** leg every value is a string, so an
    /// `integer` column declared `text` still sends `10` and still matches
    /// byte-for-byte; only the `RowDescription` tells the two apart — and a
    /// text-protocol driver (psql, node-postgres, the wasm adapter) types its
    /// output by that declaration, so a wrong one is a wrong value to every real
    /// client. On the **extended** leg the values are decoded bytes, so a declared
    /// type that is outright *incompatible* with what the server sent already fails
    /// the decode — but one that is merely **wrong yet width-compatible** (`text`
    /// where Postgres says `int4[]`, `int8` where it says `int4`) decodes cleanly and
    /// is invisible without this.
    ///
    /// The return type is a four-way [`DeclaredTypes`] and not an
    /// `Option<mismatch>` because "no columns to compare" and "no comparison
    /// happened" are different facts that an `Option` collapses into the same
    /// `None`. Collapsed, they fail **open**: a probe-side DB error or protocol
    /// desync folds into [`Expected::Error`], whose `column_types()` is empty, so
    /// the very failure that should be loudest silently disabled the check and the
    /// fixture still printed green. An empty side was likewise waved through before
    /// the count check could see it, so Postgres declaring five columns against
    /// pglet's zero read as agreement. Each outcome is now named and the caller must
    /// decide what it means.
    #[must_use]
    pub fn compare_declared_types(&self, other: &Self) -> DeclaredTypes {
        let (pg, pglet) = match (self, other) {
            (
                Self::Success {
                    column_types: a, ..
                },
                Self::Success {
                    column_types: b, ..
                },
            ) => (a, b),
            // Both engines rejected the `@test` the same way. A rejected statement
            // describes no columns, and the SQLSTATE agreement is asserted by the
            // value comparison — nothing here to check. A `CLIENT_FAULT_CODE` is
            // excluded by name: a harness fault reproducing on both connections is
            // still a harness fault, and letting it match itself would rebuild the
            // very alibi this split exists to remove.
            (Self::Error { error: pg }, Self::Error { error: pglet })
                if pg.code == pglet.code && pg.code != CLIENT_FAULT_CODE =>
            {
                return DeclaredTypes::BothErrored(pg.code.clone());
            }
            // Anything else on the error side is a harness-side fact, never
            // evidence of agreement.
            (Self::Error { error: pg }, Self::Error { error: pglet }) => {
                return DeclaredTypes::ProbeFailed(format!(
                    "probes failed differently: Postgres {} ({}), pglet {} ({})",
                    pg.code, pg.message, pglet.code, pglet.message
                ));
            }
            (Self::Error { error }, Self::Success { .. }) => {
                return DeclaredTypes::ProbeFailed(format!(
                    "Postgres probe failed: {} ({})",
                    error.code, error.message
                ));
            }
            (Self::Success { .. }, Self::Error { error }) => {
                return DeclaredTypes::ProbeFailed(format!(
                    "pglet probe failed: {} ({})",
                    error.code, error.message
                ));
            }
        };
        // Both probes ran and neither described a row — the `@test`'s last statement
        // emits no `RowDescription` (an INSERT/UPDATE/DDL with no RETURNING). Nothing
        // to assert, and structurally so: it needs no per-fixture opt-out.
        if pg.is_empty() && pglet.is_empty() {
            return DeclaredTypes::NoColumns;
        }
        // Reached only when at least one side has columns, so an empty *other* side
        // is a real divergence and lands here rather than short-circuiting above.
        if pg.len() != pglet.len() {
            return DeclaredTypes::Mismatch(format!(
                "column count differs: Postgres {} vs pglet {}",
                pg.len(),
                pglet.len()
            ));
        }
        let diffs: Vec<String> = pg
            .iter()
            .zip(pglet.iter())
            .filter(|(a, b)| a.type_oid != b.type_oid)
            .map(|(a, b)| {
                format!(
                    "{}: Postgres oid={} vs pglet oid={}",
                    a.name, a.type_oid, b.type_oid
                )
            })
            .collect();
        if diffs.is_empty() {
            DeclaredTypes::Match
        } else {
            DeclaredTypes::Mismatch(diffs.join(", "))
        }
    }

    /// Create an error result with no captured structured fields — for the callers
    /// that have none (a harness-side fault, a `FIXTURE` parse failure, the
    /// hand-rolled simple-query probe).
    #[must_use]
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Error {
            error: QueryError {
                code: code.into(),
                message: message.into(),
                fields: ErrorFields::default(),
            },
        }
    }

    /// Create an error result carrying the structured `ErrorResponse` fields the
    /// server sent — see [`QueryError::fields`].
    #[must_use]
    pub fn error_with_fields(
        code: impl Into<String>,
        message: impl Into<String>,
        fields: ErrorFields,
    ) -> Self {
        Self::Error {
            error: QueryError {
                code: code.into(),
                message: message.into(),
                fields,
            },
        }
    }

    /// The structured fields this result carries (empty for a success).
    #[must_use]
    pub fn error_fields(&self) -> ErrorFields {
        match self {
            Self::Error { error } => error.fields.clone(),
            Self::Success { .. } => ErrorFields::default(),
        }
    }

    /// Type-flexible equality check.
    /// Handles BIGINT being returned as strings by node-postgres but as numbers by Rust.
    #[must_use]
    pub fn values_match(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Success {
                    rows: a,
                    row_count: ac,
                    ..
                },
                Self::Success {
                    rows: b,
                    row_count: bc,
                    ..
                },
            ) => {
                ac == bc
                    && a.len() == b.len()
                    && a.iter().zip(b.iter()).all(|(x, y)| json_values_equal(x, y))
            }
            (Self::Error { error: a }, Self::Error { error: b }) => {
                // For errors, just check that both are errors with same code
                a.code == b.code
            }
            _ => false,
        }
    }
}

/// Compare two JSON values with type flexibility for numeric types.
/// Handles BIGINT being returned as strings by node-postgres but parsed as numbers by Rust.
fn json_values_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;

    match (a, b) {
        // Exact match
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,

        // Type-flexible: Number vs String that parses to same number
        (Value::Number(n), Value::String(s)) | (Value::String(s), Value::Number(n)) => {
            // Try integer comparison first
            if let Some(ni) = n.as_i64()
                && let Ok(si) = s.parse::<i64>()
            {
                return ni == si;
            }
            // Try float comparison
            if let Some(nf) = n.as_f64()
                && let Ok(sf) = s.parse::<f64>()
            {
                return (nf - sf).abs() < 1e-9;
            }
            false
        }

        // Arrays: compare element-wise
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| json_values_equal(x, y))
        }

        // Objects: compare key-wise
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|bv| json_values_equal(v, bv)))
        }

        // All other combinations are not equal
        _ => false,
    }
}

/// Convert a row to JSON object (extended query protocol with binary format).
///
/// # Errors
///
/// The decode error of the first column whose bytes don't fit the type its
/// `RowDescription` declared. A server that declares one type and sends another is
/// exactly the divergence this harness exists to catch, so it surfaces as a **failed
/// fixture** (named, with the rest of the run still to come) rather than the panic an
/// infallible `Row::get` would raise partway through the suite.
fn row_to_json(row: &tokio_postgres::Row) -> Result<serde_json::Value, tokio_postgres::Error> {
    let mut obj = serde_json::Map::new();

    for (i, column) in row.columns().iter().enumerate() {
        let name = column.name();
        let value = column_value_to_json(row, i, column.type_())?;
        obj.insert(name.to_string(), value);
    }

    Ok(serde_json::Value::Object(obj))
}

/// Convert a column value to JSON, matching node-postgres behavior.
/// Key differences from Rust defaults:
/// - BIGINT (INT8) is returned as string (JS precision safety)
/// - COUNT(*) returns BIGINT, so aggregates are strings too
///
/// # Errors
///
/// The decode error when the column's bytes don't fit its declared type. See
/// [`row_to_json`].
fn column_value_to_json(
    row: &tokio_postgres::Row,
    idx: usize,
    pg_type: &tokio_postgres::types::Type,
) -> Result<serde_json::Value, tokio_postgres::Error> {
    use tokio_postgres::types::Type;

    Ok(match *pg_type {
        Type::BOOL => {
            let v: Option<bool> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, serde_json::Value::Bool)
        }
        Type::INT2 => {
            let v: Option<i16> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |n| serde_json::json!(n))
        }
        Type::INT4 => {
            let v: Option<i32> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |n| serde_json::json!(n))
        }
        Type::INT8 => {
            // BIGINT: node-postgres returns as string for JS precision
            let v: Option<i64> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |n| {
                serde_json::Value::String(n.to_string())
            })
        }
        Type::FLOAT4 => {
            let v: Option<f32> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |n| serde_json::json!(n))
        }
        Type::FLOAT8 => {
            let v: Option<f64> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |n| serde_json::json!(n))
        }
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => {
            let v: Option<String> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, serde_json::Value::String)
        }
        // An OID is a number on both engines and in node-postgres. Without this arm
        // every `oid`-declared column recorded JSON `null`, so the binary leg of every
        // `pg_catalog` fixture that projects one compared `null == null`.
        Type::OID => {
            let v: Option<u32> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |n| serde_json::json!(n))
        }
        // `regtype` — a type **pglet does not model** (`pg_typeof` is `0A000` here), so
        // this arm exists for the oracle side alone: without it the recorded
        // `.expected.json` said `null` where 18.6 answers a type. Its binary form is a
        // 4-byte OID, which is what the recorded value is; the *text* leg carries the
        // type's name instead, and the two legs disagreeing is a property of the type,
        // not of the engine.
        Type::REGTYPE => {
            let v: Option<SqlOidOut> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |o| serde_json::json!(o.0))
        }
        // `"char"` — the other declaration-only type, read as the text either engine
        // would print rather than as tokio-postgres's `i8` (see [`SqlCharOut`]).
        Type::CHAR => {
            let v: Option<SqlCharOut> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |c| serde_json::Value::String(c.0))
        }
        // `void` — 18.6's `void_send` sends zero bytes, and node-postgres reads the text
        // form, `''`. Recorded as the bytes themselves read as text (see [`SqlVoidOut`]),
        // so a value that sent anything is a different string rather than the same `""`.
        Type::VOID => {
            let v: Option<SqlVoidOut> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |o| serde_json::Value::String(o.0))
        }
        Type::BYTEA => {
            let v: Option<Vec<u8>> = row.try_get(idx)?;
            v.map_or(
                serde_json::Value::Null,
                |bytes| serde_json::json!({ "type": "Buffer", "data": bytes }),
            )
        }
        Type::UUID => {
            let v: Option<uuid::Uuid> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |u| {
                serde_json::Value::String(u.to_string())
            })
        }
        // The `date` and timestamp arms, scalar and array, decode through `postgres_types`'
        // `Timestamp` / `Date` wrappers, which read the binary `infinity` sentinels as
        // `PosInfinity` / `NegInfinity` — a bare `time` type has no value for either — and
        // record them as the strings `infinity` / `-infinity`.
        Type::TIMESTAMP => {
            let v: Option<Timestamp<time::PrimitiveDateTime>> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |v| {
                serde_json::Value::String(timestamp_json_text(v))
            })
        }
        // DATE decodes to canonical `YYYY-MM-DD` text, making the binary DATE wire
        // format genuinely differential-tested (without this arm a date column
        // decodes to `null`, and the binary path validates nothing).
        Type::DATE => {
            let v: Option<Date<time::Date>> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |d| {
                serde_json::Value::String(date_json_text(d))
            })
        }
        // TIME decodes the raw int64 µs directly (via `SqlTimeOut`) into canonical
        // `HH:MM:SS[.ffffff]` text — tokio-postgres's `time::Time` can't represent
        // the `24:00:00` endpoint, so the shim reads the bytes itself. Makes the
        // binary TIME wire format genuinely differential-tested.
        Type::TIME => {
            let v: Option<SqlTimeOut> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |t| serde_json::Value::String(t.0))
        }
        // INTERVAL decodes the 16-byte wire form (via `SqlIntervalOut`) into
        // canonical `postgres`-style text, making the binary INTERVAL wire format
        // genuinely differential-tested (without this arm it decodes to `null`).
        Type::INTERVAL => {
            let v: Option<SqlIntervalOut> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |t| serde_json::Value::String(t.0))
        }
        Type::TIMESTAMPTZ => {
            let v: Option<Timestamp<OffsetDateTime>> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |dt| {
                serde_json::Value::String(timestamptz_json_text(dt))
            })
        }
        // `json` (114) and its array (199, below) are **oracle-side-only** arms, like
        // `regtype`: pglet answers `42704` for the spelling, so re-deriving this list from
        // `TypeOid` alone would drop them and break the record of an `@expect-error`
        // fixture 18.6 answers with a `json` column.
        Type::JSONB | Type::JSON => {
            let v: Option<serde_json::Value> = row.try_get(idx)?;
            v.unwrap_or(serde_json::Value::Null)
        }
        // NUMERIC is returned as a string (node-postgres does the same, for
        // precision), decoded from the binary wire form via `SqlNumericOut`.
        Type::NUMERIC => {
            let v: Option<SqlNumericOut> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |n| serde_json::Value::String(n.0))
        }
        // Arrays: decode the binary array format into a JSON array so pglet and
        // Postgres compare equal. (tokio-postgres's generic String fallback nulls
        // arrays, which is why bare array columns need an explicit decode here.)
        Type::TEXT_ARRAY | Type::VARCHAR_ARRAY | Type::BPCHAR_ARRAY | Type::NAME_ARRAY => {
            let v: Option<Vec<Option<String>>> = row.try_get(idx)?;
            string_array_to_json(v)
        }
        Type::UUID_ARRAY => {
            let v: Option<Vec<Option<uuid::Uuid>>> = row.try_get(idx)?;
            string_array_to_json(v.map(|items| {
                items
                    .into_iter()
                    .map(|u| u.map(|u| u.to_string()))
                    .collect()
            }))
        }
        Type::BOOL_ARRAY => {
            let v: Option<Vec<Option<bool>>> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |items| {
                serde_json::Value::Array(
                    items
                        .into_iter()
                        .map(|b| b.map_or(serde_json::Value::Null, serde_json::Value::Bool))
                        .collect(),
                )
            })
        }
        Type::INT2_ARRAY => {
            let v: Option<Vec<Option<i16>>> = row.try_get(idx)?;
            int_array_to_json(v)
        }
        Type::INT4_ARRAY => {
            let v: Option<Vec<Option<i32>>> = row.try_get(idx)?;
            int_array_to_json(v)
        }
        Type::INT8_ARRAY => {
            // BIGINT elements render as strings, matching node-postgres.
            let v: Option<Vec<Option<i64>>> = row.try_get(idx)?;
            string_array_to_json(v.map(|items| {
                items
                    .into_iter()
                    .map(|n| n.map(|n| n.to_string()))
                    .collect()
            }))
        }
        // NUMERIC / REAL / DOUBLE PRECISION arrays. Without these arms the generic
        // string fallback nulls the whole column, so a `numeric[]` or `real[]` value
        // compared `null == null` and the binary leg asserted **nothing** about it —
        // the array-widening fixtures were covered only by their text and declared-type
        // legs. Numeric elements render as strings for precision, like the scalar arm.
        Type::NUMERIC_ARRAY => {
            let v: Option<Vec<Option<SqlNumericOut>>> = row.try_get(idx)?;
            string_array_to_json(v.map(|items| items.into_iter().map(|n| n.map(|n| n.0)).collect()))
        }
        Type::FLOAT4_ARRAY => {
            let v: Option<Vec<Option<f32>>> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |items| {
                serde_json::Value::Array(
                    items
                        .into_iter()
                        .map(|n| n.map_or(serde_json::Value::Null, |n| serde_json::json!(n)))
                        .collect(),
                )
            })
        }
        Type::FLOAT8_ARRAY => {
            let v: Option<Vec<Option<f64>>> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |items| {
                serde_json::Value::Array(
                    items
                        .into_iter()
                        .map(|n| n.map_or(serde_json::Value::Null, |n| serde_json::json!(n)))
                        .collect(),
                )
            })
        }
        // The remaining array element types. Every one of these used to reach the
        // generic string fallback, which nulls the whole column — so the binary leg
        // compared `null == null` and asserted **nothing** about the values. Each
        // element renders exactly as its scalar arm above does, so an array column is
        // now as strongly checked as a scalar one.
        Type::BYTEA_ARRAY => {
            let v: Option<Vec<Option<Vec<u8>>>> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |items| {
                serde_json::Value::Array(
                    items
                        .into_iter()
                        .map(|b| {
                            b.map_or(
                                serde_json::Value::Null,
                                |bytes| serde_json::json!({ "type": "Buffer", "data": bytes }),
                            )
                        })
                        .collect(),
                )
            })
        }
        Type::TIMESTAMP_ARRAY => {
            let v: Option<Vec<Option<Timestamp<time::PrimitiveDateTime>>>> = row.try_get(idx)?;
            string_array_to_json(v.map(|items| {
                items
                    .into_iter()
                    .map(|dt| dt.map(timestamp_json_text))
                    .collect()
            }))
        }
        Type::TIMESTAMPTZ_ARRAY => {
            let v: Option<Vec<Option<Timestamp<OffsetDateTime>>>> = row.try_get(idx)?;
            string_array_to_json(v.map(|items| {
                items
                    .into_iter()
                    .map(|dt| dt.map(timestamptz_json_text))
                    .collect()
            }))
        }
        Type::DATE_ARRAY => {
            let v: Option<Vec<Option<Date<time::Date>>>> = row.try_get(idx)?;
            string_array_to_json(
                v.map(|items| items.into_iter().map(|d| d.map(date_json_text)).collect()),
            )
        }
        Type::TIME_ARRAY => {
            let v: Option<Vec<Option<SqlTimeOut>>> = row.try_get(idx)?;
            string_array_to_json(v.map(|items| items.into_iter().map(|t| t.map(|t| t.0)).collect()))
        }
        Type::INTERVAL_ARRAY => {
            let v: Option<Vec<Option<SqlIntervalOut>>> = row.try_get(idx)?;
            string_array_to_json(v.map(|items| items.into_iter().map(|t| t.map(|t| t.0)).collect()))
        }
        Type::JSONB_ARRAY | Type::JSON_ARRAY => {
            let v: Option<Vec<Option<serde_json::Value>>> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |items| {
                serde_json::Value::Array(items.into_iter().map(Option::unwrap_or_default).collect())
            })
        }
        // No arm — **fail the fixture**, naming the type. A type this decoder does not
        // know used to be recorded as JSON `null`, which made the binary leg of every
        // column declaring it vacuous (`null == null`) *and* wrote a false value into
        // the recorded `.expected.json`. The arm list is derived from what pglet
        // declares (`pglet_core`'s `TypeOid`), so a new declared type fails loudly here
        // until it is decoded.
        _ => {
            let v: Option<UndecodedType> = row.try_get(idx)?;
            v.map_or(serde_json::Value::Null, |_| serde_json::Value::Null)
        }
    })
}

/// Render an optional array of optional strings as a JSON array (NULL → JSON null,
/// missing array → JSON null).
/// The recorded text of a binary `timestamp`: millisecond-wide, `T`-separated, or the
/// infinity's word.
fn timestamp_json_text(v: Timestamp<time::PrimitiveDateTime>) -> String {
    match v {
        Timestamp::PosInfinity => "infinity".to_string(),
        Timestamp::NegInfinity => "-infinity".to_string(),
        Timestamp::Value(dt) => dt
            .format(&time::macros::format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]"
            ))
            .unwrap_or_default(),
    }
}

/// The recorded text of a binary `timestamptz`: RFC 3339, or the infinity's word.
fn timestamptz_json_text(v: Timestamp<OffsetDateTime>) -> String {
    match v {
        Timestamp::PosInfinity => "infinity".to_string(),
        Timestamp::NegInfinity => "-infinity".to_string(),
        Timestamp::Value(dt) => dt.format(&Rfc3339).unwrap_or_default(),
    }
}

/// The recorded text of a binary `date`: `YYYY-MM-DD`, or the infinity's word.
fn date_json_text(v: Date<time::Date>) -> String {
    match v {
        Date::PosInfinity => "infinity".to_string(),
        Date::NegInfinity => "-infinity".to_string(),
        Date::Value(d) => d
            .format(&time::macros::format_description!("[year]-[month]-[day]"))
            .unwrap_or_default(),
    }
}

fn string_array_to_json(items: Option<Vec<Option<String>>>) -> serde_json::Value {
    items.map_or(serde_json::Value::Null, |items| {
        serde_json::Value::Array(
            items
                .into_iter()
                .map(|s| s.map_or(serde_json::Value::Null, serde_json::Value::String))
                .collect(),
        )
    })
}

/// Render an optional array of optional integers as a JSON array.
fn int_array_to_json<T: Into<i64>>(items: Option<Vec<Option<T>>>) -> serde_json::Value {
    items.map_or(serde_json::Value::Null, |items| {
        serde_json::Value::Array(
            items
                .into_iter()
                .map(|n| n.map_or(serde_json::Value::Null, |n| serde_json::json!(n.into())))
                .collect(),
        )
    })
}

/// Run a fixture's setup + test and return the result, optionally **isolated** in
/// a transaction that is always rolled back.
///
/// When `isolate` is true the whole setup+test sequence runs inside
/// `BEGIN … ROLLBACK`, so the fixture leaves no persistent state (pglet rolls back
/// DDL too) — which is what lets fixtures drop their per-file `DROP TABLE IF
/// EXISTS` boilerplate and stops them contaminating each other. The `ROLLBACK`
/// runs even when the body errors (a failed statement leaves the transaction
/// aborted; `ROLLBACK` clears it so the connection is clean for the next fixture).
/// Fixtures that manage their own transactions (`-- @no-txn`) pass `isolate =
/// false`.
///
/// Uses extended query protocol (binary format) for test queries to validate
/// binary encoding. Setup uses simple query protocol for multi-statement support.
/// The **declared** column types of the text leg are captured separately, by
/// [`run_text_types`].
pub async fn run_query(
    client: &Client,
    setup: &[String],
    query: &str,
    text_mode: bool,
    params: &[FixtureParam],
    isolate: bool,
) -> Result<Expected, tokio_postgres::Error> {
    if !isolate {
        return run_query_inner(client, setup, query, text_mode, params).await;
    }
    client.simple_query("BEGIN").await?;
    let result = run_query_inner(client, setup, query, text_mode, params).await;
    // Always roll back — even on error — to clear any aborted transaction state.
    let _ = client.simple_query("ROLLBACK").await;
    result
}

/// The setup + test body of [`run_query`], without transaction wrapping.
async fn run_query_inner(
    client: &Client,
    setup: &[String],
    query: &str,
    text_mode: bool,
    params: &[FixtureParam],
) -> Result<Expected, tokio_postgres::Error> {
    run_setup(client, setup).await?;
    execute_test(client, query, text_mode, params).await
}

/// Run the fixture's setup statements (simple query, multi-statement support).
/// Not compared — schema + seed data only.
async fn run_setup(client: &Client, setup: &[String]) -> Result<(), tokio_postgres::Error> {
    for stmt in setup {
        client.simple_query(stmt).await?;
    }
    Ok(())
}

/// Execute the `@test` query in **one** wire format and return its result. The
/// last non-empty statement's rows are the result; `@param` values bind over the
/// extended protocol. Setup is *not* run here (see [`run_setup`]), so the two
/// formats of a read-only `@test` can share one setup (see [`run_query_dual`]).
///
/// On the **binary** (extended-protocol) path the result also carries the last
/// statement's declared column types, read off the prepared statement's `Describe`
/// reply — the planner walker's answer, which is what a binary client decodes
/// against. Preparing explicitly costs no extra wire traffic: `client.query(&str, …)`
/// already prepares the statement internally and throws the handle away; this keeps
/// it, so the columns are available even when the query returns **zero rows** (a
/// `Row`-derived capture would see nothing there). The text path records none — its
/// values arrive through tokio-postgres's `SimpleQueryRow`, which has already
/// discarded the `RowDescription` (that leg is [`SimpleProbe`]'s).
async fn execute_test(
    client: &Client,
    query: &str,
    text_mode: bool,
    params: &[FixtureParam],
) -> Result<Expected, tokio_postgres::Error> {
    // Build the bound-parameter slice (empty for literal-SQL fixtures). A bad
    // literal is reported as a synthetic protocol error so the fixture fails
    // loudly rather than panicking.
    let owned = match fixture_params_owned(params) {
        Ok(owned) => owned,
        Err(msg) => return Ok(Expected::error("FIXTURE", msg)),
    };
    let param_refs: Vec<&(dyn ToSql + Sync)> = owned.iter().map(AsRef::as_ref).collect();

    // Split test query into statements; the last statement's rows are the result.
    let statements = split_statements(query);

    let mut last_rows: Vec<serde_json::Value> = Vec::new();
    let mut last_row_count: u64 = 0;
    let mut last_columns: Vec<DeclaredColumn> = Vec::new();

    for stmt in &statements {
        if stmt.trim().is_empty() {
            continue;
        }
        if text_mode {
            // Simple query protocol returns results in the text wire format, so this
            // exercises pglet's text-format encoder (`value_to_bytes`) — the path the
            // binary `query()` below never reaches. All columns come back as strings.
            let rows: Vec<serde_json::Value> = client
                .simple_query(stmt.as_str())
                .await?
                .iter()
                .filter_map(|msg| match msg {
                    SimpleQueryMessage::Row(r) => Some(simple_row_to_json(r)),
                    _ => None,
                })
                .collect();
            last_row_count = rows.len() as u64;
            last_rows = rows;
        } else {
            // Extended query protocol with binary format (tokio-postgres default),
            // binding any `@param` values. Prepared explicitly so the `Describe`
            // reply's declared columns survive the call — see the fn docs.
            let statement = match declared_param_types(params) {
                Some(types) => client.prepare_typed(stmt.as_str(), &types).await?,
                None => client.prepare(stmt.as_str()).await?,
            };
            last_columns = statement_declared_columns(&statement);
            let rows = client.query(&statement, &param_refs).await?;
            last_rows = rows
                .iter()
                .map(row_to_json)
                .collect::<Result<Vec<_>, _>>()?;
            last_row_count = rows.len() as u64;
        }
    }

    Ok(Expected::success_typed(
        last_rows,
        last_row_count as usize,
        last_columns,
    ))
}

/// Run setup **once**, then capture the `@test` in *both* wire formats within a
/// single (optionally isolated) transaction — instead of re-running setup for the
/// text pass.
///
/// Valid **only for a read-only `@test`** (running it twice in one txn must not
/// change state — see [`test_is_read_only`]); a mutating test keeps the
/// two-transaction path in the caller. Returns `(binary, text)`, each converted to
/// a comparable [`Expected`]; a setup failure is reported as the result for both.
pub async fn run_query_dual(
    client: &Client,
    setup: &[String],
    query: &str,
    params: &[FixtureParam],
    isolate: bool,
) -> (Expected, Expected) {
    if isolate {
        let _ = client.simple_query("BEGIN").await;
    }
    let result = run_query_dual_body(client, setup, query, params).await;
    // Always roll back — even on error — to clear any aborted transaction state.
    if isolate {
        let _ = client.simple_query("ROLLBACK").await;
    }
    result
}

/// Setup-once + both-format capture body of [`run_query_dual`], without the
/// transaction wrapping.
async fn run_query_dual_body(
    client: &Client,
    setup: &[String],
    query: &str,
    params: &[FixtureParam],
) -> (Expected, Expected) {
    // A setup failure is the result for both formats (no test to run).
    if let Err(e) = run_setup(client, setup).await {
        let exp = to_expected(Err(e));
        return (exp.clone(), exp);
    }
    let binary = to_expected(execute_test(client, query, false, params).await);
    let text = to_expected(execute_test(client, query, true, params).await);
    (binary, text)
}

/// Whether a `@test` query is **read-only** for the dual-format capture.
///
/// Every statement must plainly lead with `SELECT`/`WITH`/`(` and contain no
/// data-modifying keyword as a whole word. Conservative by design — anything
/// uncertain is treated as *not* read-only (the safe two-transaction path;
/// correctness over speed). A `WITH … (DELETE …)` data-modifying CTE is excluded by
/// the keyword check, and `SELECT … INTO`/`FOR UPDATE` by the `INTO`/`UPDATE` words.
#[must_use]
pub fn test_is_read_only(query: &str) -> bool {
    for stmt in split_statements(query) {
        let lead = stmt.trim_start();
        if lead.is_empty() {
            continue;
        }
        let upper = lead.to_ascii_uppercase();
        let leads_ok =
            upper.starts_with("SELECT") || upper.starts_with("WITH") || upper.starts_with('(');
        if !leads_ok || contains_dml_word(&upper) {
            return false;
        }
    }
    true
}

/// Whether `upper` (already upper-cased) contains a data-modifying SQL keyword as
/// a whole word. The split treats `_` as a word char so `updated_at`/`created_by`
/// don't false-match `UPDATE`/`CREATE`.
fn contains_dml_word(upper: &str) -> bool {
    const DML: [&str; 9] = [
        "INSERT", "UPDATE", "DELETE", "CREATE", "DROP", "ALTER", "TRUNCATE", "MERGE", "INTO",
    ];
    upper
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|w| DML.contains(&w))
}

/// Capture the **declared column types** a fixture's `@test` reports over the
/// simple query protocol, on a [`SimpleProbe`] connection.
///
/// A separate, additive pass — the text leg's *values* still come from
/// [`run_query`]'s `text_mode` on the shared connection, unchanged. Only the types
/// need the probe, because tokio-postgres throws the simple query's
/// `RowDescription` away (`SimpleColumn` carries only a name), and pglet computes
/// those types through different code than the extended protocol does.
///
/// Being a second connection, it replays the fixture's setup. That is invisible for
/// a fixture whose setup is transactional, and **deliberately ignored** where it is
/// not: only [`Expected::column_types`] is read off this pass, never its rows. A
/// non-transactional setup (a sequence pglet's `ROLLBACK` doesn't restore) would
/// otherwise fail on the replay rather than on anything the engine got wrong.
pub async fn run_text_types(
    probe: &mut SimpleProbe,
    setup: &[String],
    query: &str,
    isolate: bool,
) -> Expected {
    if isolate && let Err(e) = probe.query("BEGIN").await {
        return probe_error_to_expected(&e);
    }
    let result = run_text_types_inner(probe, setup, query).await;
    // Always roll back — even on error — to clear any aborted transaction state.
    if isolate {
        let _ = probe.query("ROLLBACK").await;
    }
    match result {
        Ok(expected) => expected,
        Err(e) => probe_error_to_expected(&e),
    }
}

/// The setup + test body of [`run_text_types`], without transaction wrapping.
async fn run_text_types_inner(
    probe: &mut SimpleProbe,
    setup: &[String],
    query: &str,
) -> Result<Expected, ProbeError> {
    for stmt in setup {
        probe.query(stmt).await?;
    }
    let mut last = ProbeOutput::default();
    for stmt in split_statements(query) {
        if stmt.trim().is_empty() {
            continue;
        }
        last = probe.query(&stmt).await?;
    }
    let row_count = last.rows.len();
    let rows = last
        .rows
        .iter()
        .map(|row| probe_row_to_json(&last.columns, row))
        .collect();
    Ok(Expected::success_typed(rows, row_count, last.columns))
}

/// Convert a simple-query (text protocol) row to a JSON object of string columns.
///
/// Every value is the raw text the server sent (`None` → JSON null), so this
/// compares pglet's text-format encoder against Postgres's byte-for-byte.
fn simple_row_to_json(row: &SimpleQueryRow) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (i, column) in row.columns().iter().enumerate() {
        let value = row.get(i).map_or(serde_json::Value::Null, |s| {
            serde_json::Value::String(s.to_string())
        });
        obj.insert(column.name().to_string(), value);
    }
    serde_json::Value::Object(obj)
}

/// The [`SimpleProbe`] counterpart of [`simple_row_to_json`], kept so a failure
/// report can show what the type-capture pass actually saw.
fn probe_row_to_json(columns: &[DeclaredColumn], row: &[Option<String>]) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (i, column) in columns.iter().enumerate() {
        let value = row
            .get(i)
            .and_then(Clone::clone)
            .map_or(serde_json::Value::Null, serde_json::Value::String);
        obj.insert(column.name.clone(), value);
    }
    serde_json::Value::Object(obj)
}

/// Fold a [`ProbeError`] into a comparable [`Expected`], keeping the engine-vs-
/// harness split visible in the SQLSTATE slot (see [`CLIENT_FAULT_CODE`]).
fn probe_error_to_expected(error: &ProbeError) -> Expected {
    match error {
        ProbeError::Db { code, message } => Expected::error(code.clone(), message.clone()),
        ProbeError::Client(message) => Expected::error(CLIENT_FAULT_CODE, message.clone()),
    }
}

/// Run a query, catching errors. See [`run_query`] for `isolate`.
pub async fn run_query_catching_errors(
    client: &Client,
    setup: &[String],
    query: &str,
    text_mode: bool,
    params: &[FixtureParam],
    isolate: bool,
) -> Expected {
    to_expected(run_query(client, setup, query, text_mode, params, isolate).await)
}

/// The SQLSTATE slot's stand-in for a failure that is **the harness's**, not the
/// engine's: a decode error, a protocol parse failure, a dropped connection.
///
/// It exists so an `@expect-error` fixture cannot score green off one. Those
/// fixtures come in two strengths, and the weaker one — "Postgres accepts this
/// query, so any pglet error passes" — will happily accept whatever lands in the
/// error slot. A client-side decode failure landing there is precisely the
/// declared-vs-sent type divergence the harness exists to catch, so counting it as
/// the *expected* error turns the bug into its own alibi. Tagging it distinctly
/// lets that arm reject it by name.
pub const CLIENT_FAULT_CODE: &str = "CLIENT-FAULT";

/// Whether an [`Expected`] is a harness-side failure rather than an engine error
/// — see [`CLIENT_FAULT_CODE`].
#[must_use]
pub fn is_client_fault(expected: &Expected) -> bool {
    matches!(expected, Expected::Error { error } if error.code == CLIENT_FAULT_CODE)
}

/// Convert a `run_query`/`execute_test` result into a comparable [`Expected`],
/// extracting the SQLSTATE + message from a `DbError`.
///
/// An error carrying **no** `DbError` never reached the server's error path — it is
/// a client-side decode/protocol/connection failure — so it is tagged
/// [`CLIENT_FAULT_CODE`] rather than the old `UNKNOWN`, which an `@expect-error`
/// fixture could mistake for the error it was asserting.
fn to_expected(result: Result<Expected, tokio_postgres::Error>) -> Expected {
    match result {
        Ok(result) => result,
        Err(e) => e.as_db_error().map_or_else(
            || Expected::error(CLIENT_FAULT_CODE, e.to_string()),
            |db_err| {
                Expected::error_with_fields(
                    db_err.code().code().to_string(),
                    db_err.message().to_string(),
                    ErrorFields::from_db_error(db_err),
                )
            },
        ),
    }
}

/// A bound parameter type for the `-- @param` fixture directive.
///
/// Each maps to a tokio-postgres `ToSql` Rust type, so a `@param` fixture sends
/// the same binary-bound value a binary-protocol driver would — exercising the real
/// prepared-statement path, not literal SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureParamType {
    Bool,
    Int2,
    Int4,
    Int8,
    Float8,
    Text,
    Uuid,
    Jsonb,
    Timestamptz,
    Date,
    Numeric,
    TextArray,
    UuidArray,
    Int4Array,
}

/// A `NUMERIC` bound parameter for the differential harness.
///
/// tokio-postgres ships no built-in decimal type, so this newtype serializes
/// `PgNumeric`'s Postgres binary form (`numeric_send`) directly — the very
/// encoder the engine under test ships — so a `-- @param $N numeric …` fixture
/// binds a real numeric to both Postgres and pglet.
#[derive(Debug)]
struct SqlNumeric(pglet_sql::types::PgNumeric);

impl ToSql for SqlNumeric {
    fn to_sql(
        &self,
        _ty: &tokio_postgres::types::Type,
        out: &mut bytes::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        out.extend_from_slice(&self.0.to_wire_bytes());
        Ok(tokio_postgres::types::IsNull::No)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        *ty == tokio_postgres::types::Type::NUMERIC
    }

    tokio_postgres::types::to_sql_checked!();
}

/// Reads a `NUMERIC` column from the binary wire output for the harness.
///
/// tokio-postgres ships no `FromSql` for numeric, so decode `PgNumeric` from the
/// binary form and render its canonical text — matching node-postgres, which
/// returns numeric as a string (so the JSON comparison is byte-for-byte). Without
/// this the generic string fallback nulls numeric columns, making a numeric
/// fixture compare `null == null` and pass vacuously.
#[derive(Debug)]
struct SqlNumericOut(String);

impl<'a> tokio_postgres::types::FromSql<'a> for SqlNumericOut {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let mut buf = pglet_sql::types::BinaryReader::new(raw);
        let n = pglet_sql::types::PgNumeric::recv(&mut buf)?;
        if !buf.is_exhausted() {
            return Err("trailing bytes after a numeric wire value".into());
        }
        Ok(Self(n.to_display_string()))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        *ty == tokio_postgres::types::Type::NUMERIC
    }
}

/// Reads a `TIME` column from the binary wire output for the harness.
///
/// tokio-postgres decodes `time` into `time::Time`, which cannot represent the
/// `24:00:00` endpoint pglet accepts — so this shim reads the raw 8-byte int64 µs
/// directly and renders the engine's own canonical text (`format_time_text`),
/// matching node-postgres, which returns `time` as a string. Without it the generic
/// string fallback nulls time columns, making a time fixture compare `null == null`
/// and pass vacuously.
#[derive(Debug)]
struct SqlTimeOut(String);

impl<'a> tokio_postgres::types::FromSql<'a> for SqlTimeOut {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let bytes: [u8; 8] = raw.try_into().map_err(|_| "invalid time wire length")?;
        let micros = i64::from_be_bytes(bytes);
        Ok(Self(pglet_sql::types::format_time_text(micros)))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        *ty == tokio_postgres::types::Type::TIME
    }
}

/// Reads an `INTERVAL` column from the binary wire output for the harness.
///
/// tokio-postgres ships no `FromSql` for interval, so decode the 16-byte wire form
/// (time i64 µs → day i32 → month i32) into a `PgInterval` and render its
/// `postgres`-style text — matching node-postgres, which returns interval as a
/// string. Without this the generic string fallback nulls interval columns, making
/// an interval fixture compare `null == null` and pass vacuously.
#[derive(Debug)]
struct SqlIntervalOut(String);

impl<'a> tokio_postgres::types::FromSql<'a> for SqlIntervalOut {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let mut buf = pglet_sql::types::BinaryReader::new(raw);
        let iv = pglet_sql::types::PgInterval::recv(&mut buf)?;
        if !buf.is_exhausted() {
            return Err("trailing bytes after an interval wire value".into());
        }
        Ok(Self(pglet_sql::types::format_interval_text(&iv)))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        *ty == tokio_postgres::types::Type::INTERVAL
    }
}

/// Reads an OID-shaped column tokio-postgres has no `FromSql` for — `regtype`, whose
/// wire form is the 4-byte OID `oid` itself carries.
#[derive(Debug)]
struct SqlOidOut(u32);

impl<'a> tokio_postgres::types::FromSql<'a> for SqlOidOut {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let bytes: [u8; 4] = raw.try_into().map_err(|_| "invalid oid wire length")?;
        Ok(Self(u32::from_be_bytes(bytes)))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        *ty == tokio_postgres::types::Type::REGTYPE
    }
}

/// Reads a `"char"` column from the binary wire output for the harness.
///
/// tokio-postgres decodes `"char"` as `i8` — the byte, not the text a client prints,
/// and `charout` renders byte 0 as the empty string. Take the raw bytes instead: one
/// byte from 18.6, the value's UTF-8 from pglet (`"char"` is a `Value::Text` there), so
/// an empty `"char"` compares equal on the two engines instead of failing to decode.
///
/// A byte that is not valid UTF-8 on its own — 18.6 truncates `'é'::"char"` to the lone
/// `\303` — renders as the replacement character. That divergence is `EDGES.md` row 86
/// and no corpus catalog carries such a value.
#[derive(Debug)]
struct SqlCharOut(String);

impl<'a> tokio_postgres::types::FromSql<'a> for SqlCharOut {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(if raw.is_empty() || raw == [0] {
            String::new()
        } else {
            String::from_utf8_lossy(raw).into_owned()
        }))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        *ty == tokio_postgres::types::Type::CHAR
    }
}

/// A `void` column's binary bytes, read as text: the empty string for the zero bytes
/// `void_send` sends, and whatever else was sent otherwise.
#[derive(Debug)]
struct SqlVoidOut(String);

impl<'a> tokio_postgres::types::FromSql<'a> for SqlVoidOut {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(String::from_utf8_lossy(raw).into_owned()))
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        *ty == tokio_postgres::types::Type::VOID
    }
}

/// Accepts nothing, so `try_get` on a column whose type has no arm in
/// [`column_value_to_json`] fails with tokio-postgres's own "cannot convert between the
/// Rust type … and the Postgres type …", naming the type that needs one.
#[derive(Debug)]
struct UndecodedType;

impl<'a> tokio_postgres::types::FromSql<'a> for UndecodedType {
    fn from_sql(
        _ty: &tokio_postgres::types::Type,
        _raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self)
    }

    fn accepts(_ty: &tokio_postgres::types::Type) -> bool {
        false
    }
}

impl FixtureParamType {
    /// Parse a `-- @param` type keyword.
    fn parse(kw: &str) -> Option<Self> {
        Some(match kw {
            "bool" | "boolean" => Self::Bool,
            "int2" | "smallint" => Self::Int2,
            "int4" | "int" | "integer" => Self::Int4,
            "int8" | "bigint" => Self::Int8,
            "float8" | "double" => Self::Float8,
            "text" | "varchar" => Self::Text,
            "uuid" => Self::Uuid,
            // no `json` alias: the engine answers `42704` for that spelling.
            "jsonb" => Self::Jsonb,
            "timestamptz" | "timestamp" => Self::Timestamptz,
            "date" => Self::Date,
            "numeric" | "decimal" => Self::Numeric,
            "text[]" => Self::TextArray,
            "uuid[]" => Self::UuidArray,
            "int4[]" => Self::Int4Array,
            _ => return None,
        })
    }

    /// The type a `-- @param-declared` directive declares at `Parse` — the canonical
    /// type this keyword binds, so an alias declares its canonical type (`timestamp`
    /// declares `timestamptz`, `varchar` declares `text`).
    const fn declared_type(self) -> tokio_postgres::types::Type {
        use tokio_postgres::types::Type;
        match self {
            Self::Bool => Type::BOOL,
            Self::Int2 => Type::INT2,
            Self::Int4 => Type::INT4,
            Self::Int8 => Type::INT8,
            Self::Float8 => Type::FLOAT8,
            Self::Text => Type::TEXT,
            Self::Uuid => Type::UUID,
            Self::Jsonb => Type::JSONB,
            Self::Timestamptz => Type::TIMESTAMPTZ,
            Self::Date => Type::DATE,
            Self::Numeric => Type::NUMERIC,
            Self::TextArray => Type::TEXT_ARRAY,
            Self::UuidArray => Type::UUID_ARRAY,
            Self::Int4Array => Type::INT4_ARRAY,
        }
    }
}

/// A bound parameter from a `-- @param $N <type> <literal>` or
/// `-- @param-declared $N <type> <literal>` directive.
#[derive(Debug, Clone)]
pub struct FixtureParam {
    /// 1-based parameter index (`$N`).
    pub index: usize,
    pub ty: FixtureParamType,
    /// The literal text, or `None` for a SQL NULL.
    pub literal: Option<String>,
    /// `@param-declared`: the parameter's type is declared in the `Parse` message
    /// ([`FixtureParamType::declared_type`]) rather than left for the server to infer.
    pub declared: bool,
}

/// The `Parse` parameter types for a statement binding `params`, or `None` when no
/// parameter is `@param-declared` — the statement is then prepared with no types, as a
/// `@param` fixture is.
///
/// A parameter left undeclared beside a declared one is sent as `unknown` (705), which
/// both engines read as "infer this one": 18.6's `variable_paramref_hook` reads a slot
/// holding `UNKNOWNOID` as it reads an unspecified `0`, and pglet's wire decoder maps `705`
/// to no type.
fn declared_param_types(params: &[FixtureParam]) -> Option<Vec<tokio_postgres::types::Type>> {
    if !params.iter().any(|p| p.declared) {
        return None;
    }
    let count = params.iter().map(|p| p.index).max().unwrap_or(0);
    let mut types = vec![tokio_postgres::types::Type::UNKNOWN; count];
    for p in params.iter().filter(|p| p.declared) {
        if let Some(slot) = p.index.checked_sub(1).and_then(|i| types.get_mut(i)) {
            *slot = p.ty.declared_type();
        }
    }
    Some(types)
}

/// Parse a Postgres array text literal (`{a,b,c}`) into its elements.
fn parse_array_elems(s: &str) -> Vec<String> {
    let inner = s.trim().trim_start_matches('{').trim_end_matches('}');
    if inner.is_empty() {
        return Vec::new();
    }
    inner.split(',').map(|e| e.trim().to_string()).collect()
}

/// Parse an optional literal with `f`, tagging a failure as a `bad {kind} param`.
/// A `None` literal (a SQL NULL) stays `None` without calling `f`.
fn opt_parse<T, E>(
    lit: Option<&str>,
    kind: &str,
    f: impl Fn(&str) -> Result<T, E>,
) -> Result<Option<T>, String> {
    lit.map(|s| f(s).map_err(|_| format!("bad {kind} param: {lit:?}")))
        .transpose()
}

/// Convert a `FixtureParam` to a boxed tokio-postgres value.
///
/// Each arm boxes a concrete `Option<T>` (which is `ToSql`), so NULL and typed
/// values are both expressed without a hand-written `ToSql` impl.
fn fixture_param_to_sql(p: &FixtureParam) -> Result<Box<dyn ToSql + Sync>, String> {
    let lit = p.literal.as_deref();
    Ok(match p.ty {
        FixtureParamType::Bool => Box::new(opt_parse(lit, "bool", str::parse::<bool>)?),
        FixtureParamType::Int2 => Box::new(opt_parse(lit, "int2", str::parse::<i16>)?),
        FixtureParamType::Int4 => Box::new(opt_parse(lit, "int4", str::parse::<i32>)?),
        FixtureParamType::Int8 => Box::new(opt_parse(lit, "int8", str::parse::<i64>)?),
        FixtureParamType::Float8 => Box::new(opt_parse(lit, "float8", str::parse::<f64>)?),
        FixtureParamType::Text => Box::new(lit.map(str::to_string)),
        FixtureParamType::Uuid => Box::new(opt_parse(lit, "uuid", uuid::Uuid::parse_str)?),
        FixtureParamType::Jsonb => Box::new(opt_parse(lit, "jsonb", |s| {
            serde_json::from_str::<serde_json::Value>(s)
        })?),
        // `infinity` / `-infinity` bind the binary sentinels, through `postgres_types`'
        // wrappers, so a fixture reaches each engine's binary receive of them.
        FixtureParamType::Timestamptz => Box::new(opt_parse(lit, "timestamptz", |s| match s {
            "infinity" => Ok(Timestamp::PosInfinity),
            "-infinity" => Ok(Timestamp::NegInfinity),
            _ => OffsetDateTime::parse(s, &Rfc3339).map(Timestamp::Value),
        })?),
        FixtureParamType::Date => Box::new(opt_parse(lit, "date", |s| match s {
            "infinity" => Ok(Date::PosInfinity),
            "-infinity" => Ok(Date::NegInfinity),
            _ => time::Date::parse(
                s,
                &time::macros::format_description!("[year]-[month]-[day]"),
            )
            .map(Date::Value),
        })?),
        FixtureParamType::Numeric => Box::new(opt_parse(lit, "numeric", |s| {
            pglet_sql::types::PgNumeric::parse(s).map(SqlNumeric)
        })?),
        FixtureParamType::TextArray => Box::new(lit.map(parse_array_elems)),
        FixtureParamType::UuidArray => Box::new(opt_parse(lit, "uuid[]", |s| {
            parse_array_elems(s)
                .iter()
                .map(|e| uuid::Uuid::parse_str(e))
                .collect::<Result<Vec<_>, _>>()
        })?),
        // An element spelled `NULL` is a NULL element, which the other array kinds
        // have no spelling for.
        FixtureParamType::Int4Array => Box::new(opt_parse(lit, "int4[]", |s| {
            parse_array_elems(s)
                .iter()
                .map(|e| match e.as_str() {
                    "NULL" => Ok(None),
                    e => e.parse::<i32>().map(Some),
                })
                .collect::<Result<Vec<_>, _>>()
        })?),
    })
}

/// Build the tokio-postgres parameter reference slice from fixture params,
/// ordered by `$N`. The returned `Vec<Box<...>>` owns the values; borrow it for
/// `client.query`.
fn fixture_params_owned(params: &[FixtureParam]) -> Result<Vec<Box<dyn ToSql + Sync>>, String> {
    let mut ordered = params.to_vec();
    ordered.sort_by_key(|p| p.index);
    ordered.iter().map(fixture_param_to_sql).collect()
}

/// Parsed SQL fixture.
// One field per directive, and four of them are presence flags. A state machine would
// hide the correspondence between a field here and a `-- @` line in the file, which is
// the whole readability of this struct.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug)]
pub struct ParsedFixture {
    pub path: String,
    pub setup: Vec<String>,
    pub test: String,
    pub expect_error: Option<String>,
    /// `-- @text`: run the test query over the simple-query (text) protocol so the
    /// text-format wire encoder is checked against Postgres, not the binary path.
    pub text_mode: bool,
    /// `-- @param $N <type> <literal>`: bound parameters for the test query,
    /// ordered by `$N`. Empty for the common literal-SQL fixture.
    pub params: Vec<FixtureParam>,
    /// `-- @no-txn`: run setup+test *without* the isolating `BEGIN … ROLLBACK`
    /// wrapper. For fixtures that manage their own transactions (the
    /// `transaction/*` suite), where an outer transaction would conflict.
    pub no_txn: bool,
    /// `-- @profile-lock`: the fixture's `@test` takes a different plan under the
    /// two build profiles — accelerated lifts a predicate into an
    /// `IndexScan`/`IndexRangeScan`/`GinScan` (or reaches index selection and must
    /// decline identically), stripped runs `Filter(Scan)`. These are the fixtures
    /// the stripped-profile regression lock re-runs (`--profile-lock`).
    pub profile_lock: bool,
    /// `-- @no-profile-lock: <why>`: the justification for opting this fixture out
    /// of the `@expect-error`-over-an-index completeness rule (see
    /// `check_expect_error_index_tags` in the `test_fixtures` binary). `Some` only
    /// when the fixture carries the directive, and the text is never empty.
    pub no_profile_lock: Option<String>,
    /// `-- @no-declared-types: <why>`: the justification for opting this fixture
    /// out of the text leg's declared-column-type comparison (see
    /// `Expected::compare_declared_types`). For a fixture whose values match
    /// Postgres but whose *declared* types diverge for a reason outside the change
    /// at hand — a type pglet does not model, an aggregate whose result type it
    /// widens differently. `Some` only when the fixture carries the directive, and
    /// the text is never empty: an unexplained waiver is the silent omission
    /// wearing a directive.
    pub no_declared_types: Option<String>,
    /// `-- @no-error-fields: <why>`: the justification for opting this `@expect-error`
    /// fixture out of the structured-`ErrorResponse`-field comparison
    /// (`ErrorFields::divergences`). For a fixture where pglet legitimately sends a
    /// *different* value than 18.6 — the clearest case is a `42601` whose parser stops
    /// at an earlier token, so both engines send a `P` and the two disagree about
    /// which token is wrong. Same contract as the other opt-outs: the justification is
    /// required, because the point is to record a known divergence, not to silence one.
    pub no_error_fields: Option<String>,
    /// `-- @expect-message`: also require pglet's primary message (`M`) to equal
    /// Postgres's **byte for byte**.
    ///
    /// Opt-in rather than default-on, and the asymmetry is deliberate: `M` diverges on
    /// a long, separately-tracked list of statements (`EDGES.md`), so a default-on
    /// comparison would redden them all at once and the directive would immediately
    /// become a waiver worn by half the corpus. Opt-in makes it a **lock**: a fixture
    /// carrying it asserts that this message is now 18.6's, and reddens the day it
    /// stops being.
    pub expect_message: bool,
}

/// Why a fixture file could not be parsed as a `test_fixtures` fixture.
///
/// The distinction is load-bearing: a fixture from another directive family is
/// legitimately not ours to run (skip it), while a malformed one is a **defect**
/// that must fail the run. Absorbing the second case as a skip is how a typo'd
/// directive stays green.
#[derive(Debug, thiserror::Error)]
pub enum FixtureParseError {
    /// Belongs to another directive family — the `copy/` suite, which the
    /// `test_copy` binary parses with its own `@copy-from`/`@copy-to` grammar.
    #[error("{0}")]
    ForeignFamily(String),
    /// Malformed: an unknown directive, a missing or duplicated `@test`, a bad
    /// `@param`, …
    #[error("{0}")]
    Invalid(String),
}

impl From<String> for FixtureParseError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}

/// Directives belonging to the `copy/` fixture family (`test_copy`'s grammar).
///
/// Recognized here only so a COPY fixture reaching this parser is reported as
/// *foreign* rather than *malformed* — it is skipped, not failed. The names are
/// honored **only under `copy/`** ([`is_copy_fixture`]): `data` and `verify` are
/// ordinary English words, so matching them by name anywhere would let a stray
/// `-- @verify the ordering is stable` in any fixture turn that fixture into a
/// silent skip — the same "malformed fixture passes as green" hole the
/// unknown-directive check exists to close. A typo inside a `copy/` fixture is not
/// caught here, since the fixture is foreign from its first COPY directive on; it is
/// `test_copy`'s parser that refuses one.
const COPY_DIRECTIVES: &[&str] = &["copy-from", "copy-to", "data", "data-hex", "verify"];

/// The directive spellings this parser accepts, for the unknown-directive message.
const KNOWN_DIRECTIVES: &str = "@setup @test @text @param @param-declared @expect-error: @no-txn @profile-lock @no-profile-lock: \
     @no-declared-types: @no-error-fields: @expect-message";

/// Whether `path` names a fixture in the `copy/` family (`tests/fixtures/copy/…`).
///
/// Callers pass either the fixtures-dir-relative name or the full path, so the test
/// is on a path *segment* rather than a prefix.
fn is_copy_fixture(path: &str) -> bool {
    path.split('/').any(|segment| segment == "copy")
}

/// The body of a `-- @…` directive comment line (everything after the `@`), or
/// `None` when the line is not a directive candidate.
///
/// Two normalizations, each closing a way a fixture could silently lose a directive:
///
/// - **Spacing between `--` and `@` is not significant.** `-- @test`, `--@test`, and
///   `--   @test` are one directive. Requiring the literal `"-- @"` prefix meant
///   `--@no-txn` parsed clean with the flag unset, changing what the fixture asserts
///   with nothing to notice it.
/// - **The name character class is `[A-Za-z0-9_]`,** not just letters, so `-- @1foo`
///   and `-- @_foo` are candidates (and therefore rejected as unknown) instead of
///   falling through as inert SQL text.
///
/// A `@` followed by anything outside that class is not a candidate at all, which is
/// what keeps prose about the containment operator (`-- @> true: …`) out of the check.
///
/// `test_copy`'s fixture parser reads its own directive family through this same
/// predicate, so the two parsers agree on what a directive line is.
pub fn directive_body(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("--")?.trim_start().strip_prefix('@')?;
    rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        .then_some(rest)
}

/// Parse a fixture file content.
///
/// A comment line of the form `-- @…` must name a known directive. An
/// unrecognized one is a hard error rather than inert SQL-comment text: a
/// misspelled `-- @profile-lokc` would otherwise silently drop the fixture from
/// the set that directive selects, and the run would report green having never
/// exercised it.
pub fn parse_fixture(content: &str, path: &str) -> Result<ParsedFixture, FixtureParseError> {
    let mut setup_lines = Vec::new();
    let mut test_lines = Vec::new();
    let mut expect_error = None;
    let mut text_mode = false;
    let mut no_txn = false;
    let mut profile_lock = false;
    let mut no_profile_lock: Option<String> = None;
    let mut no_declared_types: Option<String> = None;
    let mut no_error_fields: Option<String> = None;
    let mut expect_message = false;
    let mut params: Vec<FixtureParam> = Vec::new();
    let mut in_test = false;
    let mut test_directive_count = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        // A `-- @…` line is a directive candidate; everything else is SQL text.
        if let Some(body) = directive_body(trimmed) {
            match body {
                "setup" => {
                    in_test = false;
                    continue;
                }
                "test" => {
                    test_directive_count += 1;
                    in_test = true;
                    continue;
                }
                // `@text`: compare the text-format wire output (simple query protocol).
                "text" => {
                    text_mode = true;
                    continue;
                }
                // `@no-txn`: opt out of the isolating BEGIN/ROLLBACK wrapper.
                "no-txn" => {
                    no_txn = true;
                    continue;
                }
                // `@profile-lock`: this fixture's plan differs between build profiles.
                "profile-lock" => {
                    profile_lock = true;
                    continue;
                }
                _ => {}
            }
            // `@param $N <type> <literal>`: a bound parameter for the test query.
            if let Some(rest) = body.strip_prefix("param ") {
                params.push(parse_param_directive(rest, path, false)?);
                continue;
            }
            // `@param-declared $N <type> <literal>`: the same, with the type declared in
            // the `Parse` message.
            if let Some(rest) = body.strip_prefix("param-declared ") {
                params.push(parse_param_directive(rest, path, true)?);
                continue;
            }
            if let Some(rest) = body.strip_prefix("expect-error:") {
                expect_error = Some(rest.trim().to_string());
                continue;
            }
            // `@no-profile-lock: <why>`: the justified opt-out from the
            // `@expect-error`-over-an-index completeness rule. The justification is
            // required — the point of the rule is to force a decision, and an
            // unexplained waiver is the silent omission wearing a directive.
            if let Some(rest) = body.strip_prefix("no-profile-lock:") {
                let reason = rest.trim();
                if reason.is_empty() {
                    return Err(FixtureParseError::Invalid(format!(
                        "{path}: `-- @no-profile-lock:` needs a justification after the colon"
                    )));
                }
                no_profile_lock = Some(reason.to_string());
                continue;
            }
            // `@no-declared-types: <why>`: the justified opt-out from the text leg's
            // declared-column-type comparison. Same contract as `@no-profile-lock:` —
            // the justification is required, because the point is to record a known
            // divergence, not to silence one.
            if let Some(rest) = body.strip_prefix("no-declared-types:") {
                let reason = rest.trim();
                if reason.is_empty() {
                    return Err(FixtureParseError::Invalid(format!(
                        "{path}: `-- @no-declared-types:` needs a justification after the colon"
                    )));
                }
                no_declared_types = Some(reason.to_string());
                continue;
            }
            // `@no-error-fields: <why>`: the justified opt-out from the structured
            // error-field comparison. Same required-justification contract.
            if let Some(rest) = body.strip_prefix("no-error-fields:") {
                let reason = rest.trim();
                if reason.is_empty() {
                    return Err(FixtureParseError::Invalid(format!(
                        "{path}: `-- @no-error-fields:` needs a justification after the colon"
                    )));
                }
                no_error_fields = Some(reason.to_string());
                continue;
            }
            // `@expect-message`: lock this fixture's primary message to Postgres's.
            if body.trim() == "expect-message" {
                expect_message = true;
                continue;
            }

            // No arm matched, so this names no known directive. Fail loud instead of
            // falling through to the SQL-text arms: a typo (`-- @profile-lokc`, or
            // `-- @expect-error` missing its colon) would otherwise be absorbed as an
            // inert comment and quietly change what the fixture asserts.
            let name: String = body
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            // The `copy/` family's own grammar — foreign, not malformed — but only
            // for a fixture that actually lives under `copy/`.
            if COPY_DIRECTIVES.contains(&name.as_str()) && is_copy_fixture(path) {
                return Err(FixtureParseError::ForeignFamily(format!(
                    "{path}: COPY fixture (`-- @{name}`) — run by `test_copy`"
                )));
            }
            return Err(FixtureParseError::Invalid(format!(
                "{path}: unknown directive on line {trimmed:?} (known: {KNOWN_DIRECTIVES})"
            )));
        }

        if in_test {
            test_lines.push(line.to_string());
        } else {
            setup_lines.push(line.to_string());
        }
    }

    if test_directive_count == 0 {
        return Err(FixtureParseError::Invalid(format!(
            "{path}: Missing @test directive"
        )));
    }
    if test_directive_count > 1 {
        return Err(FixtureParseError::Invalid(format!(
            "{path}: Multiple @test directives (only one allowed)"
        )));
    }

    let setup_sql = setup_lines.join("\n").trim().to_string();
    let setup_statements = split_statements(&setup_sql);
    let test_query = test_lines.join("\n").trim().to_string();

    if test_query.is_empty() {
        return Err(FixtureParseError::Invalid(format!(
            "{path}: Empty @test query"
        )));
    }

    if !params.is_empty() {
        // Bound parameters need the extended protocol (no Bind in simple query)...
        if text_mode {
            return Err(FixtureParseError::Invalid(format!(
                "{path}: @param is incompatible with @text"
            )));
        }
        // ...and a single statement, so the params bind unambiguously.
        if split_statements(&test_query)
            .iter()
            .filter(|s| !s.trim().is_empty())
            .count()
            > 1
        {
            return Err(FixtureParseError::Invalid(format!(
                "{path}: @param requires a single-statement @test query"
            )));
        }
        // Indices must be the contiguous set $1..=$N.
        params.sort_by_key(|p| p.index);
        for (i, p) in params.iter().enumerate() {
            if p.index != i + 1 {
                return Err(FixtureParseError::Invalid(format!(
                    "{path}: @param indices must be contiguous $1..$N (got ${})",
                    p.index
                )));
            }
        }
    }

    Ok(ParsedFixture {
        path: path.to_string(),
        setup: setup_statements,
        test: test_query,
        expect_error,
        text_mode,
        params,
        no_txn,
        profile_lock,
        no_profile_lock,
        no_declared_types,
        no_error_fields,
        expect_message,
    })
}

/// Parse the body of a `-- @param $N <type> <literal>` directive.
///
/// `NULL` (case-insensitive) is a typed NULL; otherwise the literal is the
/// remaining text with one optional layer of surrounding single quotes stripped.
fn parse_param_directive(rest: &str, path: &str, declared: bool) -> Result<FixtureParam, String> {
    let mut parts = rest.splitn(3, char::is_whitespace);
    let index_tok = parts.next().unwrap_or("");
    let type_tok = parts.next().unwrap_or("");
    let literal_tok = parts.next().unwrap_or("").trim();

    let index: usize = index_tok
        .strip_prefix('$')
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("{path}: bad @param index {index_tok:?} (expected $N)"))?;
    let ty = FixtureParamType::parse(type_tok)
        .ok_or_else(|| format!("{path}: unknown @param type {type_tok:?}"))?;

    let literal = if literal_tok.eq_ignore_ascii_case("null") {
        None
    } else if let Some(inner) = literal_tok
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
    {
        Some(inner.to_string())
    } else {
        Some(literal_tok.to_string())
    };

    Ok(FixtureParam {
        index,
        ty,
        literal,
        declared,
    })
}

/// Split SQL into statements by semicolon (respecting quotes and `--` comments).
//
// A `;` inside a string literal or a `--` line comment is not a separator. Block
// comments (`/* */`) are not handled — fixtures don't use them.
fn split_statements(sql: &str) -> Vec<String> {
    // Indexed over `char`s (not bytes) so look-ahead for a dollar-quote tag is
    // UTF-8-correct.
    let chars: Vec<char> = sql.chars().collect();
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_line_comment = false;
    // The open dollar-quote tag (`$$`, `$foo$`) when inside a dollar-quoted body —
    // a `;` there must NOT split (a `DO $$ … ; … $$` block holds inner `;`s).
    let mut dollar_tag: Option<String> = None;

    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // A `--` line comment runs to a line feed or a carriage return, as `scan.l`'s
        // does; copy it verbatim, don't split on it.
        if in_line_comment {
            current.push(c);
            if c == '\n' || c == '\r' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        // Inside a dollar-quoted body: copy verbatim until the matching close tag.
        if let Some(tag) = &dollar_tag {
            if c == '$'
                && let Some(t) = read_dollar_tag(&chars, i)
                && &t == tag
            {
                current.push_str(&t);
                i += t.chars().count();
                dollar_tag = None;
                continue;
            }
            current.push(c);
            i += 1;
            continue;
        }
        match c {
            '\'' if !in_double_quote => {
                // Check for escape
                in_single_quote = !in_single_quote;
                current.push(c);
                i += 1;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                current.push(c);
                i += 1;
            }
            '-' if !in_single_quote && !in_double_quote && chars.get(i + 1) == Some(&'-') => {
                in_line_comment = true;
                current.push_str("--");
                i += 2; // consume both dashes
            }
            '$' if !in_single_quote && !in_double_quote => {
                if let Some(t) = read_dollar_tag(&chars, i) {
                    current.push_str(&t);
                    i += t.chars().count();
                    dollar_tag = Some(t);
                } else {
                    current.push(c);
                    i += 1;
                }
            }
            ';' if !in_single_quote && !in_double_quote => {
                let stmt = current.trim().to_string();
                if !stmt.is_empty() {
                    statements.push(stmt);
                }
                current.clear();
                i += 1;
            }
            _ => {
                current.push(c);
                i += 1;
            }
        }
    }

    let last = current.trim().to_string();
    if !last.is_empty() {
        statements.push(last);
    }

    statements
}

/// Read a dollar-quote tag (`$$`, `$foo$`) starting at `chars[start]` (a `$`),
/// returning the full tag string if one is present, else `None`. A tag is `$`, an
/// optional identifier (letter/underscore start, then alphanumerics/underscores),
/// then a closing `$`. The leading-letter rule keeps a positional parameter (`$1`)
/// from being misread as a tag open.
fn read_dollar_tag(chars: &[char], start: usize) -> Option<String> {
    let mut j = start + 1;
    let mut first = true;
    while let Some(&c) = chars.get(j) {
        if c == '$' {
            return Some(chars[start..=j].iter().collect());
        }
        let ok = if first {
            c.is_ascii_alphabetic() || c == '_'
        } else {
            c.is_ascii_alphanumeric() || c == '_'
        };
        if !ok {
            return None;
        }
        first = false;
        j += 1;
    }
    None
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "a failed setup in a test is a test failure, and the message is the diagnosis"
)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fixture() {
        let content = r"-- @setup
DROP TABLE IF EXISTS users CASCADE;
CREATE TABLE users (id INT, name TEXT);

-- @test
SELECT * FROM users;
";

        let fixture = parse_fixture(content, "test.sql").unwrap();
        assert_eq!(fixture.setup.len(), 2);
        assert!(fixture.setup[0].contains("DROP TABLE"));
        assert!(fixture.setup[1].contains("CREATE TABLE"));
        assert_eq!(fixture.test, "SELECT * FROM users;");
        assert!(fixture.expect_error.is_none());
        assert!(!fixture.text_mode);
    }

    #[test]
    fn split_statements_respects_dollar_quotes() {
        // A `;` inside a `$$ … $$` body must not split — a DO block holds inner
        // `;`s. Two DO blocks + a trailing statement → three statements.
        let sql = "DO $$ BEGIN CREATE TYPE t AS ENUM ('a'); EXCEPTION WHEN duplicate_object THEN NULL; END $$;\n\
                   DO $$ BEGIN CREATE TYPE t AS ENUM ('a'); EXCEPTION WHEN duplicate_object THEN NULL; END $$;\n\
                   CREATE TABLE x (id INT);";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 3, "got {stmts:?}");
        assert!(stmts[0].starts_with("DO $$") && stmts[0].contains("END $$"));
        assert!(stmts[2].starts_with("CREATE TABLE"));
        // A positional parameter (`$1`) is not a dollar-quote open.
        let p = split_statements("SELECT $1; SELECT $2;");
        assert_eq!(p.len(), 2, "got {p:?}");
    }

    #[test]
    fn test_parse_fixture_text_directive() {
        let content = "-- @test\n-- @text\nSELECT '\\xff'::bytea AS b;\n";
        let fixture = parse_fixture(content, "t.sql").unwrap();
        assert!(fixture.text_mode);
        // The directive line is consumed, not left in the query.
        assert_eq!(fixture.test, "SELECT '\\xff'::bytea AS b;");
    }

    #[test]
    fn test_parse_fixture_param_directive() {
        let content = "-- @param $1 uuid 11111111-1111-1111-1111-111111111111\n\
                       -- @param $2 text NULL\n\
                       -- @test\n\
                       SELECT name FROM t WHERE id = $1 AND k = $2;\n";
        let fixture = parse_fixture(content, "p.sql").unwrap();
        assert_eq!(fixture.params.len(), 2);
        assert_eq!(fixture.params[0].index, 1);
        assert_eq!(fixture.params[0].ty, FixtureParamType::Uuid);
        assert_eq!(
            fixture.params[0].literal.as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        // NULL literal parses to a typed NULL.
        assert_eq!(fixture.params[1].ty, FixtureParamType::Text);
        assert_eq!(fixture.params[1].literal, None);
    }

    #[test]
    fn test_parse_fixture_param_rejects_non_contiguous() {
        let content = "-- @param $1 int4 1\n-- @param $3 int4 3\n-- @test\nSELECT $1, $3;\n";
        assert!(parse_fixture(content, "p.sql").is_err());
    }

    #[test]
    fn test_parse_fixture_param_rejects_text_mode() {
        let content = "-- @param $1 int4 1\n-- @text\n-- @test\nSELECT $1;\n";
        assert!(parse_fixture(content, "p.sql").is_err());
    }

    /// Wrap `line` in a minimal but otherwise-valid fixture, so a parse failure can
    /// only come from `line` itself.
    fn fixture_with_line(line: &str) -> String {
        format!("-- @test\nSELECT 1;\n{line}\n")
    }

    #[test]
    fn directive_spacing_between_dashes_and_at_is_not_significant() {
        // `--@profile-lock` parsed clean with the flag *unset*, so a fixture could
        // silently lose a directive to a missing space.
        for spelling in ["-- @profile-lock", "--@profile-lock", "--   @profile-lock"] {
            let fixture = parse_fixture(&format!("{spelling}\n-- @test\nSELECT 1;\n"), "p.sql")
                .unwrap_or_else(|e| panic!("{spelling}: {e}"));
            assert!(fixture.profile_lock, "{spelling}");
        }
        let fixture = parse_fixture("--@no-txn\n--@text\n-- @test\nSELECT 1;\n", "p.sql").unwrap();
        assert!(fixture.no_txn && fixture.text_mode);
    }

    #[test]
    fn unknown_directives_are_rejected() {
        for line in [
            "-- @foo",
            "      -- @foo",
            "-- @foo\r",
            "-- @TEXT",
            "-- @test extra words",
            "-- @expect-error",
            "-- @no-txn2",
            "-- @profile-lokc",
            // Non-letter identifier starts: `is_ascii_alphabetic` let these through.
            "-- @1foo",
            "-- @_foo",
            // …and each of the above with the spacing bypass.
            "--@foo",
            "--@1foo",
        ] {
            let err = parse_fixture(&fixture_with_line(line), "p.sql")
                .expect_err(&format!("{line:?} should be rejected"));
            assert!(
                matches!(&err, FixtureParseError::Invalid(m) if m.contains("unknown directive")),
                "{line:?} gave {err}"
            );
        }
    }

    #[test]
    fn containment_operator_prose_is_not_a_directive() {
        // `@>` is not `@` + an identifier character, so prose about containment
        // (`types/jsonb_build_object.sql`) stays inert SQL-comment text.
        for line in ["-- @> true: single-key subset", "--@> false: disjoint"] {
            parse_fixture(&fixture_with_line(line), "p.sql")
                .unwrap_or_else(|e| panic!("{line:?}: {e}"));
        }
    }

    #[test]
    fn copy_directives_are_foreign_only_under_copy() {
        // `data` and `verify` are ordinary English words. Matching them by name
        // anywhere turned any fixture containing one into a silent *skip*.
        for line in [
            "-- @verify the ordering is stable",
            "-- @data 1,2",
            "-- @data-hex 00",
        ] {
            let outside = parse_fixture(&fixture_with_line(line), "select/x.sql")
                .expect_err(&format!("{line:?} outside copy/ should be rejected"));
            assert!(
                matches!(&outside, FixtureParseError::Invalid(_)),
                "{line:?} gave {outside}"
            );
            let inside = parse_fixture(&fixture_with_line(line), "copy/x.sql")
                .expect_err(&format!("{line:?} under copy/ should be foreign"));
            assert!(
                matches!(&inside, FixtureParseError::ForeignFamily(_)),
                "{line:?} gave {inside}"
            );
        }
    }

    #[test]
    fn no_profile_lock_requires_a_justification() {
        let fixture = parse_fixture(
            "-- @no-profile-lock: DDL `@test` — index selection never runs\n\
             -- @expect-error: duplicate\n\
             -- @test\n\
             CREATE INDEX i ON t (k);\n",
            "d.sql",
        )
        .unwrap();
        assert_eq!(
            fixture.no_profile_lock.as_deref(),
            Some("DDL `@test` — index selection never runs")
        );
        // An unexplained waiver is the silent omission wearing a directive.
        assert!(parse_fixture(&fixture_with_line("-- @no-profile-lock:"), "d.sql").is_err());
        assert!(parse_fixture(&fixture_with_line("-- @no-profile-lock:   "), "d.sql").is_err());
    }

    #[test]
    fn no_declared_types_requires_a_justification() {
        let fixture = parse_fixture(
            &fixture_with_line("-- @no-declared-types: pglet models no `name` type"),
            "d.sql",
        )
        .unwrap();
        assert_eq!(
            fixture.no_declared_types.as_deref(),
            Some("pglet models no `name` type")
        );
        // Same contract as `@no-profile-lock:` — recording a known divergence, not
        // silencing one.
        assert!(parse_fixture(&fixture_with_line("-- @no-declared-types:"), "d.sql").is_err());
        assert!(parse_fixture(&fixture_with_line("-- @no-declared-types:   "), "d.sql").is_err());
    }

    #[test]
    fn no_error_fields_requires_a_justification() {
        let fixture = parse_fixture(
            &fixture_with_line("-- @no-error-fields: 18.6 sends no `P` on this route"),
            "e.sql",
        )
        .unwrap();
        assert_eq!(
            fixture.no_error_fields.as_deref(),
            Some("18.6 sends no `P` on this route")
        );
        // The third opt-out under the same contract: an unexplained waiver is the
        // silent omission wearing a directive.
        assert!(parse_fixture(&fixture_with_line("-- @no-error-fields:"), "e.sql").is_err());
        assert!(parse_fixture(&fixture_with_line("-- @no-error-fields:   "), "e.sql").is_err());
    }

    #[test]
    fn expect_message_is_a_bare_flag() {
        let fixture = parse_fixture(&fixture_with_line("-- @expect-message"), "e.sql").unwrap();
        assert!(fixture.expect_message);
        assert!(
            !parse_fixture(&fixture_with_line("-- @text"), "e.sql")
                .unwrap()
                .expect_message
        );
        // A typo is a hard parse error, not an inert comment that silently drops the
        // lock.
        assert!(parse_fixture(&fixture_with_line("-- @expect-mesage"), "e.sql").is_err());
    }

    /// A client-side failure must be distinguishable from an engine error, so the
    /// "Postgres accepts it, any pglet error passes" arm of an `@expect-error`
    /// fixture cannot score green off a harness decode bug.
    #[test]
    fn client_fault_is_not_an_engine_error() {
        assert!(is_client_fault(&Expected::error(
            CLIENT_FAULT_CODE,
            "decode failed"
        )));
        assert!(!is_client_fault(&Expected::error("42883", "no operator")));
        assert!(!is_client_fault(&Expected::success(Vec::new(), 0)));
    }

    /// A helper for the declared-type cases below: a success declaring `oids` (names
    /// are positional, and the comparison keys on the OID). Leg-agnostic, like the
    /// comparison it feeds.
    fn typed(oids: &[u32]) -> Expected {
        Expected::success_typed(
            Vec::new(),
            0,
            oids.iter()
                .enumerate()
                .map(|(i, &type_oid)| DeclaredColumn {
                    name: format!("c{i}"),
                    type_oid,
                })
                .collect(),
        )
    }

    /// The declared-type check must not fail **open**. Each way it can decline to
    /// assert is a separate outcome, because the old `Option<String>` collapsed
    /// "nothing to compare" and "the comparison never ran" into the same `None` —
    /// so a probe error, or one side describing no columns at all, printed green.
    #[test]
    fn declared_types_distinguish_agreement_from_absence() {
        // The assertion ran and held / found a real divergence.
        assert_eq!(
            typed(&[23, 25]).compare_declared_types(&typed(&[23, 25])),
            DeclaredTypes::Match
        );
        assert!(matches!(
            typed(&[23]).compare_declared_types(&typed(&[25])),
            DeclaredTypes::Mismatch(_)
        ));

        // Neither side described a row (an INSERT/DDL `@test`) — structurally
        // nothing to assert, and no per-fixture opt-out needed to say so.
        assert_eq!(
            typed(&[]).compare_declared_types(&typed(&[])),
            DeclaredTypes::NoColumns
        );

        // The regression: one side empty is a *divergence*, not a non-assertion.
        // Postgres describing five columns against pglet's zero used to short-
        // circuit to `None` before the count check could ever see it.
        assert!(matches!(
            typed(&[23, 25, 16, 20, 701]).compare_declared_types(&typed(&[])),
            DeclaredTypes::Mismatch(_)
        ));
        assert!(matches!(
            typed(&[]).compare_declared_types(&typed(&[23])),
            DeclaredTypes::Mismatch(_)
        ));

        // Both engines rejected the `@test` identically: a rejected statement
        // describes no columns and the SQLSTATE is asserted elsewhere.
        assert_eq!(
            Expected::error("42704", "no such index")
                .compare_declared_types(&Expected::error("42704", "no such index")),
            DeclaredTypes::BothErrored("42704".to_string())
        );

        // A probe that failed is the absence of evidence, never evidence of
        // agreement — including a harness fault that reproduces on both sides,
        // which must not be allowed to match itself into a pass.
        assert!(matches!(
            Expected::error("42704", "boom").compare_declared_types(&typed(&[23])),
            DeclaredTypes::ProbeFailed(_)
        ));
        assert!(matches!(
            typed(&[23]).compare_declared_types(&Expected::error("42704", "boom")),
            DeclaredTypes::ProbeFailed(_)
        ));
        assert!(matches!(
            Expected::error("42704", "a").compare_declared_types(&Expected::error("42883", "b")),
            DeclaredTypes::ProbeFailed(_)
        ));
        assert!(matches!(
            Expected::error(CLIENT_FAULT_CODE, "desync")
                .compare_declared_types(&Expected::error(CLIENT_FAULT_CODE, "desync")),
            DeclaredTypes::ProbeFailed(_)
        ));
    }

    #[test]
    fn test_split_statements() {
        let sql = "SELECT 1; SELECT 'a;b'; SELECT \"c;d\"";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 3);
        assert_eq!(stmts[0], "SELECT 1");
        assert_eq!(stmts[1], "SELECT 'a;b'");
        assert_eq!(stmts[2], "SELECT \"c;d\"");
    }

    #[test]
    fn test_split_statements_ignores_semicolon_in_line_comment() {
        // A `;` inside a `--` comment must not split the statement: this is one
        // statement, not two (the regression that broke the bytea fixture).
        let one = split_statements("SELECT 1 -- a comment with a ; in it\n + 1;");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0], "SELECT 1 -- a comment with a ; in it\n + 1");

        // Real terminators still split; the comment's `;` adds no extra splits.
        let two = split_statements("SELECT 1; -- c1; c2\nSELECT 2;");
        assert_eq!(two.len(), 2);
        assert_eq!(two[0], "SELECT 1");
        assert_eq!(two[1], "-- c1; c2\nSELECT 2");
    }

    #[test]
    fn test_split_statements_ends_a_line_comment_at_a_carriage_return() {
        // As the engines' lexer does: the `;` after the carriage return terminates, so a
        // fixture line carrying a raw `\r` splits where both engines split it.
        let two = split_statements("SELECT 1 --c\r; SELECT 2;");
        assert_eq!(two, vec!["SELECT 1 --c", "SELECT 2"]);
        // The comment's own `;` still splits nothing.
        let one = split_statements("SELECT 1 --c;\r+ 1;");
        assert_eq!(one, vec!["SELECT 1 --c;\r+ 1"]);
    }

    // ---- the oracle override -------------------------------------------------

    /// The default is the whole safety story: unset means *exactly* what the
    /// harness did before the knob existed, byte for byte, so nobody's habits
    /// change under them.
    #[test]
    fn an_unset_override_is_the_historical_default() {
        let config = oracle_config(None).expect("the default is well-formed");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 5432);
        assert_eq!(config.user, "postgres");
        assert_eq!(config.database, "postgres");
        assert_eq!(config.password, None);
        assert_eq!(
            config.connection_string(),
            "host=127.0.0.1 port=5432 user=postgres dbname=postgres"
        );
    }

    /// Every field a reader consumes moves together: the conninfo Shape A hands
    /// tokio-postgres and the shell-outs pass positionally, and the split fields
    /// `SimpleProbe` sends in its startup message.
    #[test]
    fn a_url_override_reaches_every_oracle_reader() {
        let config = oracle_config(Some("postgres://alice:hunter2@10.0.0.7:6543/scratch"))
            .expect("a well-formed URL parses");
        assert_eq!(config.host, "10.0.0.7");
        assert_eq!(config.port, 6543);
        assert_eq!(config.user, "alice");
        assert_eq!(config.database, "scratch");
        assert_eq!(config.password, Some("hunter2"));
        assert_eq!(
            config.connection_string(),
            "host=10.0.0.7 port=6543 user=alice dbname=scratch password=hunter2"
        );
    }

    /// The `key=value` conninfo spelling parses too, because the parsing is
    /// tokio-postgres's own and it accepts both — so the harness cannot disagree
    /// with the client library about what a value meant.
    #[test]
    fn the_keyword_value_spelling_is_accepted_too() {
        let config = oracle_config(Some("host=127.0.0.1 port=5555 user=bob dbname=wb"))
            .expect("a conninfo string parses");
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 5555);
        assert_eq!(config.user, "bob");
        assert_eq!(config.database, "wb");
        assert_eq!(config.password, None);
    }

    /// The wrong implementation this guards against, and the reason the variable
    /// exists at all: a typo that falls back to the default would run `DROP SCHEMA
    /// public CASCADE` against `127.0.0.1:5432` — the database the contributor was
    /// steering away from. Every rejection names the variable, so the message says
    /// which knob to fix.
    #[test]
    fn a_malformed_override_fails_loud_rather_than_defaulting() {
        for bad in [
            "not a url at all",
            "postgres://",
            "postgres://127.0.0.1:5432/db",
            "postgres://user@127.0.0.1:5432",
            "postgres://user@/var/run/postgresql/db",
            "postgres://user@a.example:5432,b.example:5432/db",
        ] {
            let error = oracle_config(Some(bad)).expect_err("a malformed override must be refused");
            assert!(
                error.contains(ORACLE_URL_VAR),
                "the refusal of `{bad}` must name the variable: {error}"
            );
        }
    }

    /// A pglet server is not the oracle. The override must not follow the harness
    /// into the servers it spawns: those live on a dynamically chosen loopback
    /// port and take any credentials, and pointing them elsewhere would compare
    /// the oracle against someone else's server.
    #[test]
    fn the_pglet_server_config_is_untouched_by_the_override() {
        let config = pglet_config(5479);
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 5479);
        assert_eq!(config.user, "postgres");
        assert_eq!(config.database, "postgres");
        assert_eq!(config.password, None);
        assert_eq!(
            config.connection_string(),
            "host=127.0.0.1 port=5479 user=postgres dbname=postgres"
        );
    }

    /// The corpus limit the override runs into, as a predicate rather than as a
    /// surprise: a fixture that selects `current_user` / `current_database()`
    /// compares against a snapshot holding the *default* spelling, so an override
    /// moving either name makes those fixtures report a stale expectation, and
    /// regenerating under it would commit the local spelling. Moving only the
    /// host or the port is free.
    #[test]
    fn the_session_identity_limit_is_a_predicate_on_the_names_only() {
        let elsewhere = oracle_config(Some("postgres://postgres@10.0.0.7:6543/postgres"))
            .expect("a well-formed URL parses");
        assert!(oracle_identity_is_default(&elsewhere));

        for moved in [
            "postgres://postgres@127.0.0.1:5432/scratch",
            "postgres://alice@127.0.0.1:5432/postgres",
        ] {
            let config = oracle_config(Some(moved)).expect("a well-formed URL parses");
            assert!(
                !oracle_identity_is_default(&config),
                "`{moved}` moves the session identity"
            );
        }
    }

    /// The shell-out gates' argv is the config's conninfo and nothing else — in
    /// particular no `PGPASSWORD` literal, which used to be set to `postgres` for
    /// the pglet targets as well as the oracle and would have outranked an
    /// override's own credential.
    #[test]
    fn the_shell_outs_take_the_config_positionally() {
        let oracle = oracle_config(Some("postgres://alice:hunter2@10.0.0.7:6543/scratch"))
            .expect("a well-formed URL parses");
        let mut command = std::process::Command::new("pg_dump");
        apply_conninfo(&mut command, &oracle);
        let args: Vec<&std::ffi::OsStr> = command.get_args().collect();
        assert_eq!(
            args,
            ["host=10.0.0.7 port=6543 user=alice dbname=scratch password=hunter2"]
        );
        assert_eq!(command.get_envs().count(), 0, "no PGPASSWORD literal");

        let mut pglet = std::process::Command::new("psql");
        apply_conninfo(&mut pglet, &pglet_config(5479));
        let args: Vec<&std::ffi::OsStr> = pglet.get_args().collect();
        assert_eq!(
            args,
            ["host=127.0.0.1 port=5479 user=postgres dbname=postgres"]
        );
        assert_eq!(pglet.get_envs().count(), 0);
    }

    /// The name of the variable the child test below is re-executed under. Spelled
    /// as the literal it must be, not as [`ORACLE_URL_VAR`]: a rename of the
    /// constant that forgets the environment is exactly what this pair catches, and
    /// reading the constant here would rename the assertion along with the bug.
    const ORACLE_URL_VAR_LITERAL: &str = "PGLET_PG_URL";

    /// The URL the child observes. A closed port and a database nothing creates:
    /// the child only reads the resolved config, and must never be able to reach a
    /// server even if a later edit made it try.
    const CHILD_OVERRIDE_URL: &str = "postgres://alice:s3cret@127.0.0.1:1/override_probe";

    /// The *binding* between the environment and the harness's resolved oracle —
    /// the one thing the `oracle_config` tests above cannot reach, because they all
    /// pass a `&str` in by hand.
    ///
    /// Without it, an edit that reads a mistyped variable name leaves every test in
    /// this file green while the harness resets the schema of the default
    /// `127.0.0.1:5432` database under docs saying the knob steers it elsewhere —
    /// the accident the knob exists to prevent. [`POSTGRES_CONFIG`] is a
    /// `LazyLock`, so a process observes exactly one value of the variable; the
    /// observation therefore happens in a **child** process, re-executing this test
    /// binary's own ignored test with the variable set.
    #[test]
    fn the_environment_variable_is_what_the_resolved_oracle_reads() {
        let exe = std::env::current_exe().expect("the test binary knows its own path");
        let output = std::process::Command::new(exe)
            .args([
                "--exact",
                "--ignored",
                "--nocapture",
                "tests::the_child_observes_the_override_and_leaves_pglet_alone",
            ])
            .env(ORACLE_URL_VAR_LITERAL, CHILD_OVERRIDE_URL)
            .output()
            .expect("the test binary re-executes");
        assert!(
            output.status.success(),
            "the child observed something other than `{CHILD_OVERRIDE_URL}`:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        // The child exiting 0 having run *nothing* would be a vacuous pass — a
        // renamed test, a filter that stopped matching.
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("1 passed"),
            "the child ran no test:\n{stdout}"
        );
    }

    /// Runs in the child process of the test above, under
    /// [`ORACLE_URL_VAR`] set to [`CHILD_OVERRIDE_URL`]. Ignored so the parent's `--ignored`
    /// filter is what selects it and an ordinary run skips it.
    ///
    /// Two observations, and the second is not implied by the first: the oracle the
    /// harness resolved **is** what the variable named, and the config for a pglet
    /// server the harness spawns is **not** — deriving one from the other would
    /// point every spawned-server connection at the override's host and compare the
    /// oracle against someone else's server.
    #[test]
    #[ignore = "re-executed with the override set by the test above"]
    fn the_child_observes_the_override_and_leaves_pglet_alone() {
        let oracle = &*POSTGRES_CONFIG;
        assert_eq!(oracle.host, "127.0.0.1");
        assert_eq!(oracle.port, 1);
        assert_eq!(oracle.user, "alice");
        assert_eq!(oracle.database, "override_probe");
        assert_eq!(oracle.password, Some("s3cret"));
        assert_eq!(
            oracle.connection_string(),
            "host=127.0.0.1 port=1 user=alice dbname=override_probe password=s3cret"
        );

        let pglet = pglet_config(5479);
        assert_eq!(pglet.host, "127.0.0.1");
        assert_eq!(pglet.port, 5479);
        assert_eq!(pglet.user, "postgres");
        assert_eq!(pglet.database, "postgres");
        assert_eq!(pglet.password, None);
    }

    /// A value the parse accepted must reach every consumer unaltered. libpq's
    /// `key = value` grammar ends an unquoted value at the first whitespace, so a
    /// password holding a space rendered a conninfo `psql` refuses (`missing "="`)
    /// — reported by the harness as an unreachable server — and one holding
    /// ` dbname=other` retargeted the connection silently.
    #[test]
    fn a_rendered_conninfo_quotes_what_libpq_would_otherwise_resplit() {
        // Space, single quote, backslash, and a keyword that would retarget.
        let config = oracle_config(Some(
            "postgres://alice@10.0.0.7:6543/scratch?password=se%20c%27r%5Ct%20dbname%3Dother",
        ))
        .expect("a well-formed URL parses");
        assert_eq!(config.password, Some("se c'r\\t dbname=other"));
        assert_eq!(
            config.connection_string(),
            "host=10.0.0.7 port=6543 user=alice dbname=scratch \
             password='se c\\'r\\\\t dbname=other'"
        );
        // And the rendering round-trips through the client library's own parser:
        // whatever `psql` does with it, the tokio-postgres leg reads back exactly
        // the value that was parsed in.
        let reparsed: tokio_postgres::Config = config
            .connection_string()
            .parse()
            .expect("the rendering re-parses");
        assert_eq!(
            reparsed.get_password(),
            Some(b"se c'r\\t dbname=other".as_slice())
        );
        assert_eq!(reparsed.get_dbname(), Some("scratch"));

        // The unquoted forms are left alone, so the default is unchanged.
        assert_eq!(quote_conninfo_value("postgres"), "postgres");
        assert_eq!(quote_conninfo_value("127.0.0.1"), "127.0.0.1");
        // Empty is quoted too: libpq skips the blanks after `=` and reads the NEXT
        // keyword as the value, so a bare `password= dbname=x` swallows `dbname`
        // and lands on the database libpq defaults to (the user's name) — measured,
        // not inferred.
        assert_eq!(quote_conninfo_value(""), "''");
    }

    /// The health check's diagnosis is the client library's *chain*, not its
    /// outermost category: `tokio_postgres::Error` displays as "db error" /
    /// "error connecting to server", and the sentence that separates a server
    /// that is down from a database that does not exist is one `source` below it.
    #[test]
    fn a_health_failure_reports_the_whole_error_chain() {
        #[derive(Debug)]
        struct Layer {
            text: &'static str,
            cause: Option<Box<Self>>,
        }
        impl std::fmt::Display for Layer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.text)
            }
        }
        impl std::error::Error for Layer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                let cause: &Self = self.cause.as_deref()?;
                Some(cause)
            }
        }

        let deep = Layer {
            text: "db error",
            cause: Some(Box::new(Layer {
                text: "FATAL: database \"scratch\" does not exist",
                cause: None,
            })),
        };
        assert_eq!(
            error_chain(&deep),
            "db error: FATAL: database \"scratch\" does not exist"
        );
        // A cause-free error is its own text, with no trailing separator.
        assert_eq!(
            error_chain(&Layer {
                text: "db error",
                cause: None
            }),
            "db error"
        );
    }

    /// libpq trims a conninfo's surrounding whitespace and the URL form does not,
    /// so a variable with a stray trailing space used to connect to `scratch` while
    /// the harness reported it had resolved `scratch ` — the report disagreeing
    /// with the connection.
    #[test]
    fn surrounding_whitespace_in_the_override_is_trimmed_before_it_is_read() {
        let config = oracle_config(Some("  postgres://alice@10.0.0.7:6543/scratch \n"))
            .expect("a well-formed URL parses once trimmed");
        assert_eq!(config.database, "scratch");
        assert_eq!(
            config.connection_string(),
            "host=10.0.0.7 port=6543 user=alice dbname=scratch"
        );
    }
}
