//! True power-loss durability driver — the half of the gate that `kill -9` can't
//! reach.
//!
//! `test_crash` (`SIGKILL`) proves **acked ⇒ durable** against *process* death:
//! a commit whose ack the client received passed fsync #2, so it survives the
//! process dying. But `kill -9` *cannot* lose fsynced-but-page-cached data the way
//! pulled power can — the kernel's page cache outlives the process. The one
//! remaining durability question is therefore **power loss**: data the device
//! acked but that never reached the platter, evaporating when the lights go out.
//!
//! Reproducing that needs a **fault-injecting block device** (`dm-flakey`), which
//! needs **root** + device-mapper, so this driver is only ever invoked by
//! `tools/durability/power_loss_test.sh` (the privileged half), which sets up a
//! loopback + `dm-flakey` device on a throwaway VM and drives this binary around a
//! simulated power cut. Run directly with no setup, it **honestly skips** (prints
//! why, exits 0) rather than faking a pass.
//!
//! ## Modes
//!
//! - `commit --data-dir <dir> --record <file> [--n <count>]` — spawn `pglet_server`
//!   over `<dir>` (on the healthy device), drive `<count>` **awaited** (acked)
//!   single-row commits, and record the acked count + a checksum to `<file>` (which
//!   the script keeps on a *non-faulty* filesystem). The script trips the device
//!   fault *after* this returns, so this phase is itself clean.
//! - `verify --data-dir <dir> --record <file>` — respawn `pglet_server` over the
//!   *post-fault* `<dir>` and assert the recover-or-loud invariant through the live
//!   server: (1) **acked ⇒ durable** — every recorded acked row is present; (2) the
//!   surviving set is a **valid committed prefix** — contiguous `1..=len`, no gaps,
//!   dupes, reorderings, or torn values. Any violation exits non-zero (the finding).
//!
//! Because the `commit` phase awaits *every* insert, all recorded rows are acked,
//! so the bar in `verify` is the strictest form of the contract: the full prefix
//! `1..=n` must survive a clean reopen. A short prefix, a torn value, or a failed
//! reopen each means an acked commit was lost to the simulated power loss — a real
//! durability bug, surfaced as a non-zero exit.

// Test-harness binary: `.expect()`/`panic!` surface setup failures loudly, the
// right behavior for a test runner rather than threading `Result` everywhere.
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::Duration;

use pglet_testing::server::{connect, find_open_port, hard_kill, spawn_server, wait_for_ready};
use pglet_tests::{DYNAMIC_PORT_END, PGLET_TEST_PORT};
use tokio_postgres::Client;

/// Printed (then exit 0) when the driver is run without the privileged device
/// setup — an honest skip, never a fake pass.
const HONEST_SKIP: &str = "[power-loss] SKIP: this driver needs root + a fault-injecting block \
device (dm-flakey), so it is invoked by `tools/durability/power_loss_test.sh` on a throwaway VM \
(which sets up the loopback + dm-flakey device and passes --data-dir/--record). Running this \
binary directly is a no-op — run the script as root instead.";

/// Default number of acked commits driven by the `commit` phase
/// (override via `--n` or `PGLET_POWER_LOSS_N`).
const DEFAULT_N: i32 = 200;

/// The selected mode + its already-validated flags.
#[derive(Debug)]
enum Cmd {
    Commit {
        data_dir: PathBuf,
        record: PathBuf,
        n: i32,
    },
    Verify {
        data_dir: PathBuf,
        record: PathBuf,
    },
}

/// The value following `name` in `args`, if present (`--flag value` form).
fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Parse argv into a [`Cmd`], or `None` when the invocation isn't the
/// script-driven one (no recognized mode, or `--data-dir`/`--record` absent) — the
/// signal to skip honestly rather than touch a non-existent device.
fn parse_args(args: &[String]) -> Option<Cmd> {
    let mode = args.get(1)?;
    // Both flags are required in every real (script-driven) invocation; their
    // absence is what distinguishes "run directly by a human / CI" → honest skip.
    let data_dir = PathBuf::from(flag_value(args, "--data-dir")?);
    let record = PathBuf::from(flag_value(args, "--record")?);
    match mode.as_str() {
        "commit" => {
            let n = flag_value(args, "--n")
                .or_else(|| std::env::var("PGLET_POWER_LOSS_N").ok())
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_N);
            Some(Cmd::Commit {
                data_dir,
                record,
                n,
            })
        }
        "verify" => Some(Cmd::Verify { data_dir, record }),
        _ => None,
    }
}

/// Spawn the server over `data_dir` and wait until it accepts connections, with a
/// few retries so a just-released `flock` doesn't flake the reopen. Returns an
/// `Err` (not a panic) on failure so `verify` can report "store failed to reopen"
/// as a finding rather than a backtrace.
async fn spawn_ready(data_dir: &Path) -> Result<(Child, u16), String> {
    for attempt in 0..5 {
        let port = find_open_port(PGLET_TEST_PORT..DYNAMIC_PORT_END);
        let server = spawn_server(port, Some(data_dir));
        if wait_for_ready(port).await {
            return Ok((server, port));
        }
        eprintln!("[power-loss] server not ready (attempt {attempt}), retrying...");
        hard_kill(server);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!(
        "store failed to reopen over {} after the power-loss fault (image may be torn or unrecoverable)",
        data_dir.display()
    ))
}

/// The surviving ids, pulled through the live server (`SELECT id FROM t ORDER BY
/// id`). A reopen/scan error is a *loud* outcome reported as `Err`; a non-`Int4`
/// value decodes to the sentinel `i32::MIN` so a torn value fails
/// [`is_valid_prefix`] rather than masquerading as a valid id — the same contract
/// as `pglet_testing::durability::surviving_ids`, but over the live wire.
async fn surviving_ids(client: &Client) -> Result<Vec<i32>, String> {
    let rows = client
        .query("SELECT id FROM t ORDER BY id", &[])
        .await
        .map_err(|e| format!("scan after reopen failed (image may be torn): {e}"))?;
    Ok(rows
        .iter()
        .map(|row| row.try_get::<_, i32>(0).unwrap_or(i32::MIN))
        .collect())
}

/// A valid recovered state is exactly the contiguous prefix `[1, 2, …, j]` for some
/// `0 ≤ j ≤ n` — no gaps, duplicates, reorderings, out-of-range, or torn
/// (`i32::MIN`) values. The live-server analog of
/// `pglet_testing::durability::is_valid_prefix`, kept identical in shape so the two
/// halves of the gate can't drift on what "recover-or-loud" means.
fn is_valid_prefix(ids: &[i32], n: i32) -> bool {
    if ids.len() as i32 > n {
        return false;
    }
    ids.iter().enumerate().all(|(i, &id)| id == i as i32 + 1)
}

/// FNV-1a over `1..=n` — a cheap marker so the `verify` phase can detect a corrupt
/// or stale record file (the count and its checksum must agree).
fn fnv_checksum(n: i32) -> u64 {
    (1..=n).fold(0xcbf2_9ce4_8422_2325_u64, |h, i| {
        (h ^ i as u64).wrapping_mul(0x100_0000_01b3_u64)
    })
}

/// Write the acked row count + its checksum to the (non-faulty) record file.
fn write_record(path: &Path, n: i32) -> std::io::Result<()> {
    std::fs::write(
        path,
        format!("acked_rows {n}\nchecksum {:016x}\n", fnv_checksum(n)),
    )
}

/// The acked count read back from the record file, after a checksum self-check.
fn read_record(path: &Path) -> Result<i32, String> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| format!("read record {}: {e}", path.display()))?;
    let mut acked: Option<i32> = None;
    let mut checksum: Option<u64> = None;
    for line in body.lines() {
        let mut it = line.split_whitespace();
        match (it.next(), it.next()) {
            (Some("acked_rows"), Some(v)) => acked = v.parse().ok(),
            (Some("checksum"), Some(v)) => checksum = u64::from_str_radix(v, 16).ok(),
            _ => {}
        }
    }
    let acked = acked.ok_or_else(|| format!("record {} missing acked_rows", path.display()))?;
    let checksum = checksum.ok_or_else(|| format!("record {} missing checksum", path.display()))?;
    if checksum != fnv_checksum(acked) {
        return Err(format!(
            "record {} checksum mismatch — file corrupt or stale",
            path.display()
        ));
    }
    Ok(acked)
}

/// `commit` phase — drive `n` awaited (acked) single-row commits on the healthy
/// device, record the acked count, then stop the server cleanly.
///
/// Each `INSERT` is awaited, so its ack means it passed fsync #2 (the commit point)
/// *before* we record it — by pglet's contract it is durable. The phase leaves no
/// in-flight or un-fsynced state, so stopping the server loses nothing; the *fault*
/// is the device, injected by the script after this returns.
async fn mode_commit(data_dir: &Path, record: &Path, n: i32) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("create data dir: {e}"))?;
    let (server, port) = spawn_ready(data_dir).await?;

    // Run the workload capturing any error WITHOUT early-returning, then always
    // reap the server — an early `?` here would leak the child, and the leaked
    // server keeps the data-dir mounted, tripping the script's later `umount`.
    let outcome = drive_commits(port, record, n).await;

    // Stop the server. acked ⇒ durable already holds across process death (proven
    // by `test_crash`), so this introduces no data loss — the device fault is the
    // only variable under test.
    hard_kill(server);
    outcome?;
    println!(
        "[power-loss] commit: {n} acked rows written + recorded to {}",
        record.display()
    );
    Ok(())
}

/// The `commit`-phase workload, factored out so [`mode_commit`] can always
/// `hard_kill` the server even when a step fails (no leaked child holding the
/// data-dir mount).
async fn drive_commits(port: u16, record: &Path, n: i32) -> Result<(), String> {
    let client = connect(port).await;
    client
        .simple_query("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .map_err(|e| format!("create table: {e}"))?;

    for id in 1..=n {
        client
            .execute("INSERT INTO t VALUES ($1)", &[&id])
            .await
            .map_err(|e| format!("insert {id}: {e}"))?;
    }

    // Record what survival must include BEFORE stopping the server, so the verify
    // phase (post-fault, fresh process) knows exactly what was acked.
    write_record(record, n).map_err(|e| format!("write record: {e}"))?;
    Ok(())
}

/// `verify` phase — reopen the post-fault store and assert the recover-or-loud
/// invariant through the live server. Returns `Err` (→ non-zero exit) on any
/// violation: a lost acked commit, a torn/non-prefix recovery, or a failed reopen.
async fn mode_verify(data_dir: &Path, record: &Path) -> Result<(), String> {
    let acked = read_record(record)?;

    let (server, port) = spawn_ready(data_dir).await?;
    let client = connect(port).await;
    let scanned = surviving_ids(&client).await;
    hard_kill(server);
    let ids = scanned?;

    // Contract 1 — prefix-or-loud, never torn: the survivors are a contiguous
    // prefix of `1..=acked`, with no gaps, dupes, reorderings, or torn values.
    if !is_valid_prefix(&ids, acked) {
        return Err(format!(
            "torn / non-prefix recovery: {} surviving ids are not a valid committed prefix of 1..={acked}",
            ids.len()
        ));
    }
    // Contract 2 — acked ⇒ durable: because every insert was awaited, all `acked`
    // rows are acked, so the *full* prefix must survive. A short prefix means an
    // acked commit was lost to the simulated power loss — a durability violation.
    if ids.len() as i32 != acked {
        return Err(format!(
            "acked commit lost to power loss: only {} of {acked} acked rows survived",
            ids.len()
        ));
    }

    println!(
        "[power-loss] verify: all {acked} acked rows survived the simulated power loss — clean prefix recovery"
    );
    Ok(())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(cmd) = parse_args(&args) else {
        // No device / not under the script → honest skip (exit 0), never a fake pass.
        println!("{HONEST_SKIP}");
        return;
    };

    let result = match cmd {
        Cmd::Commit {
            data_dir,
            record,
            n,
        } => mode_commit(&data_dir, &record, n).await,
        Cmd::Verify { data_dir, record } => mode_verify(&data_dir, &record).await,
    };

    if let Err(e) = result {
        eprintln!("[power-loss] FAIL: {e}");
        std::process::exit(1);
    }
}
