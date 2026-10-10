//! Real-process kill-`-9` durability + soak test.
//!
//! The in-process fault-injection tests (`crates/pglet/tests/crash.rs`) prove
//! recovery over a hand-driven `BlockIo`; this drives the *real* production
//! binary on a *real* file and pulls the rug out from under it with `SIGKILL`.
//! It is the OS-level half of the durability gate (`PLAN.md` § 3).
//!
//! Run with: `cargo run -p pglet_tests --bin test_crash`
//!
//! Self-contained: builds the server, spawns it on an open port over a temp
//! `--data-dir`, drives commits via tokio-postgres, `kill -9`s the process,
//! restarts over the *same* data dir, and asserts durability. The spawn / wait /
//! connect / kill primitives come from `pglet_testing::server`.
//!
//! ## The two invariants
//!
//! 1. **Acked ⇒ durable.** A commit whose `CommandComplete` the client received
//!    has passed fsync #2 (the commit point), so it must survive a hard kill.
//!    Phase A awaits each commit, then kills, then asserts every acked row is
//!    still there after restart.
//! 2. **Atomic recovery.** A commit interrupted by the kill is all-or-nothing:
//!    after restart the row count is exactly the pre- or post-commit value,
//!    never a partial/torn set, and the store always reopens cleanly. Phase B
//!    fires a commit and kills at a randomized delay to land kills both before
//!    and after the commit point.
//!
//! Phase C repeats Phase B's mid-commit kill over rows of 20 to 200 KB, each held in
//! a multi-page overflow chain, so kills land while a chain's pages are being
//! written: after restart every surviving row must read back byte-identical to what
//! was sent, and the count must again be the pre- or post-commit value.
//!
//! Restarting over the same `--data-dir` also exercises the data-dir lock's
//! recovery: the killed process's `flock` is released by the kernel on death, so
//! the fresh process can re-acquire it. That property is asserted directly in
//! `crates/pglet_server/tests/data_dir_lock.rs`; here it is a precondition of
//! every restart below.

// Test-harness binary: `.expect()`/`panic!` surface setup failures loudly, the
// right behavior for a test runner rather than threading `Result` everywhere.
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::Duration;

use pglet_testing::rng::Xorshift;
use pglet_testing::server::{
    build_server, connect, find_open_port, hard_kill, spawn_server, wait_for_ready,
};
use pglet_tests::{DYNAMIC_PORT_END, PGLET_TEST_PORT};
use tokio_postgres::Client;

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Spawn the server over `data_dir` and wait until it accepts connections.
/// Retries the spawn a few times so a just-killed predecessor's `flock`, if not
/// yet released, doesn't flake the restart.
async fn spawn_ready(data_dir: &Path) -> (Child, u16) {
    for attempt in 0..5 {
        let port = find_open_port(PGLET_TEST_PORT..DYNAMIC_PORT_END);
        let server = spawn_server(port, Some(data_dir));
        if wait_for_ready(port).await {
            return (server, port);
        }
        eprintln!("[crash] server not ready (attempt {attempt}), retrying...");
        hard_kill(server);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("server failed to become ready over {}", data_dir.display());
}

/// `SELECT COUNT(*)` as i64 (`COUNT` is int8).
async fn count(client: &Client) -> i64 {
    client
        .query_one("SELECT COUNT(*) FROM t", &[])
        .await
        .expect("count failed")
        .get(0)
}

/// `SELECT id ORDER BY id` — used to assert the surviving rows are exactly the
/// contiguous prefix `1..=expected`, i.e. no gaps, dupes, or torn values.
async fn ids(client: &Client) -> Vec<i32> {
    client
        .query("SELECT id FROM t ORDER BY id", &[])
        .await
        .expect("id scan failed")
        .iter()
        .map(|row| row.get::<_, i32>(0))
        .collect()
}

fn assert_contiguous(ids: &[i32], expected_len: i64, ctx: &str) {
    assert_eq!(ids.len() as i64, expected_len, "{ctx}: row count mismatch");
    for (i, &id) in ids.iter().enumerate() {
        assert_eq!(
            id,
            i as i32 + 1,
            "{ctx}: torn/non-contiguous id at position {i} (got {id})"
        );
    }
}

/// Phase A — **acked ⇒ durable**. Each iteration commits a batch, *waits for the
/// ack*, optionally pauses, then `kill -9`s and restarts. Every acked row must
/// be present after recovery.
async fn phase_acked_survival(
    data_dir: &Path,
    iters: u64,
    rng: &mut Xorshift,
) -> Result<(), String> {
    let (mut server, mut port) = spawn_ready(data_dir).await;
    let client = connect(port).await;
    client
        .simple_query("CREATE TABLE t (id INT PRIMARY KEY)")
        .await
        .map_err(|e| format!("create table: {e}"))?;

    let mut expected: i64 = 0;
    for iter in 0..iters {
        let batch = 1 + rng.below(20) as i64;
        let values: Vec<String> = (expected + 1..=expected + batch)
            .map(|n| format!("({n})"))
            .collect();
        let sql = format!("INSERT INTO t VALUES {}", values.join(", "));

        let client = connect(port).await;
        // Await the ack: this commit is durable past fsync #2 before we kill.
        client
            .execute(&sql, &[])
            .await
            .map_err(|e| format!("iter {iter} insert: {e}"))?;
        expected += batch;

        // Sometimes pause after the ack before killing, to vary the window.
        if rng.below(2) == 0 {
            tokio::time::sleep(Duration::from_millis(rng.below(10))).await;
        }
        drop(client);
        hard_kill(server);

        let restarted = spawn_ready(data_dir).await;
        server = restarted.0;
        port = restarted.1;
        let client = connect(port).await;

        let got = count(&client).await;
        if got != expected {
            hard_kill(server);
            return Err(format!(
                "iter {iter}: acked commit lost after kill -9 — expected {expected} rows, got {got}"
            ));
        }
        assert_contiguous(
            &ids(&client).await,
            expected,
            &format!("phase A iter {iter}"),
        );
        eprintln!("[crash] phase A iter {iter}: {expected} acked rows survived kill -9");
    }

    hard_kill(server);
    Ok(())
}

/// Phase B — **atomic recovery under mid-commit kill**. Fire a fixed-size commit
/// on a background task, then `kill -9` after a randomized delay so the kill
/// lands both before and after the commit point across iterations. After restart
/// the count must be exactly the pre- or post-commit value (never partial), and
/// an *acked* commit must have survived.
async fn phase_midcommit_atomic(
    data_dir: &Path,
    iters: u64,
    start_rows: i64,
    rng: &mut Xorshift,
) -> Result<(), String> {
    const BATCH: i64 = 64;

    let (mut server, mut port) = spawn_ready(data_dir).await;
    let mut current = start_rows;

    for iter in 0..iters {
        let lo = current + 1;
        let hi = current + BATCH;
        let values: Vec<String> = (lo..=hi).map(|n| format!("({n})")).collect();
        let sql = format!("INSERT INTO t VALUES {}", values.join(", "));

        // Fire the commit on a background task; don't await it here.
        let client = connect(port).await;
        let insert = tokio::spawn(async move { client.execute(&sql, &[]).await });

        // Kill at a randomized delay so we straddle the commit point: a small
        // base so the statement is reliably in flight, then a spread wide enough
        // to land both before and after fsync #2 across iterations (the absolute
        // commit latency is fsync-bound and storage-dependent, so the window is
        // generous on purpose).
        tokio::time::sleep(Duration::from_millis(2 + rng.below(120))).await;
        hard_kill(server);

        // Did the client receive the ack before the process died?
        let acked = matches!(insert.await, Ok(Ok(_)));

        let restarted = spawn_ready(data_dir).await;
        server = restarted.0;
        port = restarted.1;
        let client = connect(port).await;

        let got = count(&client).await;
        let before_ok = got == current;
        let after_ok = got == current + BATCH;
        if !before_ok && !after_ok {
            hard_kill(server);
            return Err(format!(
                "iter {iter}: non-atomic recovery — count {got} is neither {current} nor {} (acked={acked})",
                current + BATCH
            ));
        }
        if acked && !after_ok {
            hard_kill(server);
            return Err(format!(
                "iter {iter}: acked mid-commit insert lost — count {got}, expected {} (acked)",
                current + BATCH
            ));
        }
        current = got;
        assert_contiguous(
            &ids(&client).await,
            current,
            &format!("phase B iter {iter}"),
        );
        eprintln!(
            "[crash] phase B iter {iter}: kill straddled commit (acked={acked}); {current} rows, atomic + clean"
        );
    }

    hard_kill(server);
    Ok(())
}

/// The body Phase C writes for row `id`: 20 to 200 KB of letters derived from the id,
/// so a row read back can be checked against what was sent without keeping it.
fn large_body(id: i64) -> String {
    let mut rng = Xorshift::new(id as u64 ^ 0x5151_5151_5151_5151);
    let len = 20_000 + rng.below(180_000) as usize;
    (0..len)
        .map(|_| char::from(b'a' + rng.below(26) as u8))
        .collect()
}

/// Phase C — **atomic recovery of overflow chains under mid-commit kill**. As Phase
/// B, over a table whose rows each span a multi-page overflow chain: every row that
/// survives a restart must read back byte-identical, and the count must be the
/// pre- or post-commit value.
async fn phase_large_rows_midcommit(
    data_dir: &Path,
    iters: u64,
    rng: &mut Xorshift,
) -> Result<(), String> {
    const BATCH: i64 = 4;

    let (mut server, mut port) = spawn_ready(data_dir).await;
    connect(port)
        .await
        .simple_query("CREATE TABLE big (id INT, body TEXT)")
        .await
        .map_err(|e| format!("create table: {e}"))?;
    let mut current: i64 = 0;

    for iter in 0..iters {
        let values: Vec<String> = (current + 1..=current + BATCH)
            .map(|n| format!("({n}, '{}')", large_body(n)))
            .collect();
        let sql = format!("INSERT INTO big VALUES {}", values.join(", "));
        let client = connect(port).await;
        let insert = tokio::spawn(async move { client.simple_query(&sql).await });
        // The statement carries about half a megabyte, so the window runs longer
        // than Phase B's.
        tokio::time::sleep(Duration::from_millis(2 + rng.below(250))).await;
        hard_kill(server);
        let acked = matches!(insert.await, Ok(Ok(_)));

        let restarted = spawn_ready(data_dir).await;
        server = restarted.0;
        port = restarted.1;
        let client = connect(port).await;
        let rows = client
            .query("SELECT id, body FROM big ORDER BY id", &[])
            .await
            .map_err(|e| format!("iter {iter}: scan after restart: {e}"))?;
        let got = rows.len() as i64;
        if got != current && got != current + BATCH {
            hard_kill(server);
            return Err(format!(
                "iter {iter}: non-atomic recovery — {got} rows, neither {current} nor {} (acked={acked})",
                current + BATCH
            ));
        }
        if acked && got != current + BATCH {
            hard_kill(server);
            return Err(format!(
                "iter {iter}: an acked commit of long rows was lost"
            ));
        }
        for (i, row) in rows.iter().enumerate() {
            let id: i32 = row.get(0);
            let body: String = row.get(1);
            if i64::from(id) != i as i64 + 1 || body != large_body(i64::from(id)) {
                hard_kill(server);
                return Err(format!(
                    "iter {iter}: row {id} did not read back as written ({} bytes)",
                    body.len()
                ));
            }
        }
        current = got;
        eprintln!(
            "[crash] phase C iter {iter}: kill straddled a long-row commit (acked={acked}); {current} rows read back whole"
        );
    }

    hard_kill(server);
    Ok(())
}

fn scratch_dir() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("pglet_crash_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create scratch data dir");
    path
}

#[tokio::main]
async fn main() {
    eprintln!("[crash] Building pglet_server...");
    if !build_server() {
        eprintln!("[crash] Build failed!");
        std::process::exit(1);
    }

    let seed = env_u64("PGLET_CRASH_SEED", 0x9E37_79B9_7F4A_7C15);
    let iters = env_u64("PGLET_CRASH_ITERS", 6);
    let mut rng = Xorshift::new(seed);
    eprintln!(
        "[crash] seed={seed} iters={iters} (override via PGLET_CRASH_SEED / PGLET_CRASH_ITERS)"
    );

    let data_dir = scratch_dir();
    println!("=== Crash / kill -9 durability suite ===\n");

    let mut failed = 0;

    println!("--- Phase A: acked => durable across kill -9 ---");
    match phase_acked_survival(&data_dir, iters, &mut rng).await {
        Ok(()) => println!("✓ acked commits survive kill -9"),
        Err(e) => {
            eprintln!("✗ acked-survival: {e}");
            failed += 1;
        }
    }

    // Carry the surviving row count into phase B.
    let start_rows = {
        let (server, port) = spawn_ready(&data_dir).await;
        let client = connect(port).await;
        let n = count(&client).await;
        hard_kill(server);
        n
    };

    println!("\n--- Phase B: atomic recovery under mid-commit kill -9 ---");
    match phase_midcommit_atomic(&data_dir, iters, start_rows, &mut rng).await {
        Ok(()) => println!("✓ mid-commit kills recover atomically + clean"),
        Err(e) => {
            eprintln!("✗ mid-commit-atomic: {e}");
            failed += 1;
        }
    }

    println!("\n--- Phase C: long rows' overflow chains under mid-commit kill -9 ---");
    let large_dir = data_dir.join("large");
    std::fs::create_dir_all(&large_dir).expect("create the long-row data dir");
    match phase_large_rows_midcommit(&large_dir, iters, &mut rng).await {
        Ok(()) => println!("✓ mid-commit kills of long rows recover atomically + whole"),
        Err(e) => {
            eprintln!("✗ long-row mid-commit: {e}");
            failed += 1;
        }
    }

    let _ = std::fs::remove_dir_all(&data_dir);

    println!("\n=== Results ===");
    println!("Failed: {failed}");
    if failed > 0 {
        std::process::exit(1);
    }
}
