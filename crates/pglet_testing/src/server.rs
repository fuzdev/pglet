//! Spawn / wait / connect / hard-kill harness for the native `pglet_server`.
//!
//! Builds the server once, spawns it on an open port (optionally over a
//! `--data-dir` for persistence), waits for it to accept connections, and — for
//! crash tests — `SIGKILL`s it. Shared by the `test_concurrent` and `test_crash`
//! harnesses; assumes the workspace root is the current directory (the binary is
//! `target/debug/pglet_server`).

use std::net::TcpListener;
use std::ops::Range;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tokio_postgres::{Client, NoTls};

/// The first open TCP port in `range` on `127.0.0.1`.
pub fn find_open_port(range: Range<u16>) -> u16 {
    let (lo, hi) = (range.start, range.end);
    for port in range {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no open port found in range {lo}-{hi}");
}

/// Whether `PGLET_NO_INDEX_ACCEL` asks for the stripped (`--no-default-features`)
/// server profile.
///
/// **Every** harness that builds `pglet_server` itself must consult this rather than
/// hard-coding a plain `cargo build -p pglet_server`. A harness that skips it silently
/// runs the *accelerated* binary under `PGLET_NO_INDEX_ACCEL=1`, so the stripped
/// profile's differential run reports green without ever having exercised the stripped
/// engine — which is how a real accelerated-vs-stripped result divergence stayed
/// invisible (a class of it is `EDGES.md` row 56).
#[must_use]
pub fn stripped_profile_requested() -> bool {
    std::env::var_os("PGLET_NO_INDEX_ACCEL").is_some()
}

/// `cargo build -p pglet_server`; returns whether it succeeded (inherits stdio so
/// build output is visible).
///
/// When `PGLET_NO_INDEX_ACCEL` is set the server is built with the stripped
/// "testing" profile (`--no-default-features` drops only `index_acceleration` —
/// `file_store` rides the server's dependency lines). An index is intended to
/// change only a query's speed, never its result, so a differential run against
/// the stripped server must stay byte-identical: this is the profile's own
/// differential run. Measured exceptions stand against that intent — `EDGES.md`'s
/// header names them — and no fixture is known to reach one, which is why this run
/// stays green over them.
pub fn build_server() -> bool {
    let mut cmd = Command::new("cargo");
    cmd.args(["build", "-p", "pglet_server"]);
    if stripped_profile_requested() {
        cmd.arg("--no-default-features");
    }
    cmd.stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("run cargo build")
        .success()
}

/// Spawn `pglet_server` on `port`, persisting to `data_dir` if given (otherwise
/// in-memory). Stdio is silenced.
pub fn spawn_server(port: u16, data_dir: Option<&Path>) -> Child {
    let mut cmd = Command::new("target/debug/pglet_server");
    cmd.args(["--port", &port.to_string()]);
    if let Some(dir) = data_dir {
        cmd.args(["--data-dir", &dir.to_string_lossy()]);
    }
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn pglet_server")
}

/// Poll until the server on `port` accepts a connection (up to ~5s).
pub async fn wait_for_ready(port: u16) -> bool {
    let conn_str =
        format!("host=127.0.0.1 port={port} user=postgres dbname=postgres connect_timeout=1");
    for _ in 0..50 {
        if let Ok((client, conn)) = tokio_postgres::connect(&conn_str, NoTls).await {
            tokio::spawn(conn);
            drop(client);
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Connect to the server on `port`. The driver task is spawned and errors out
/// silently when the server is killed (expected in crash tests).
pub async fn connect(port: u16) -> Client {
    let conn_str = format!("host=127.0.0.1 port={port} user=postgres dbname=postgres");
    let (client, connection) = tokio_postgres::connect(&conn_str, NoTls)
        .await
        .expect("connect to pglet");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

/// `SIGKILL` (std `Child::kill` on Unix) — no graceful shutdown, modelling power
/// loss — then reap, so the kernel releases the data-dir `flock` before a restart.
pub fn hard_kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Spawn `pglet_server` on the first open port in `range` (persisting to `data_dir`
/// if given) and wait until it accepts connections, retrying the spawn a few times.
///
/// The retry loop absorbs a just-killed predecessor's `flock` that the kernel has
/// not yet released, so a restart over the same `--data-dir` doesn't flake. Returns
/// the child + the chosen port, or an `Err` after 5 failed attempts (so a caller
/// can report "store failed to reopen" as a finding rather than panic). This is the
/// shared shape the crash/soak harnesses drive their restarts through.
pub async fn spawn_ready(
    range: Range<u16>,
    data_dir: Option<&Path>,
) -> Result<(Child, u16), String> {
    for attempt in 0..5 {
        let port = find_open_port(range.clone());
        let server = spawn_server(port, data_dir);
        if wait_for_ready(port).await {
            return Ok((server, port));
        }
        eprintln!("[spawn_ready] server not ready (attempt {attempt}), retrying...");
        hard_kill(server);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!(
        "server failed to become ready after 5 attempts (data_dir={data_dir:?})"
    ))
}
