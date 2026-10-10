//! The `--data-dir` single-holder guard, driven through real processes.
//!
//! Two servers over one directory would each open the single-file double-meta
//! store and each believe it owned the meta slots, so the guard is a
//! correctness property of the store and not merely a convenience. It is a
//! kernel `flock` (`pglet_core::file_lock`), which buys the second half of what
//! is asserted here: a holder that dies without unwinding — the SIGKILL of a
//! crash test, an OOM kill, a power-cut VM — releases it anyway, and the file it
//! leaves behind with its pid still in it blocks nothing.
//!
//! Neither test is here because `FileLock` itself needs child processes for it.
//! The *refusal* is provable in one process, and is proved there:
//! `flock(2)` locks the open file description, a second `open(2)` of the same
//! path makes a new description, and `pglet_core::file_lock`'s own
//! `a_second_acquire_is_refused_with_the_holder_pid` asserts the refusal without
//! spawning anything. (Only a `dup`/`fork` duplicate re-locks the *same*
//! description, and `FileLock::acquire` never produces one.) What each test here
//! adds is what only a real `pglet_server` can show:
//!
//! - the refusal test, that the server **wires** the lock into `--data-dir`
//!   startup at all, exits rather than serving, and puts the holder's pid in
//!   front of the operator;
//! - the SIGKILL test, the **death** half — a process cannot SIGKILL its own
//!   test runner and keep testing, so a child process is the only way to show
//!   it.
//!
//! Hence real children — and hence [`Server`], because a failing
//! assertion between a spawn and a kill would otherwise leak a live server that
//! outlives the run and holds a port and a directory. (Measured: it did, on the
//! deliberate red-on-revert run that proved these two tests are not vacuous.)

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{BufRead as _, BufReader, Read as _};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use pglet_testing::db::ScratchDir;

/// How long a server gets to report that it is listening: a ceiling for a loaded
/// box, not an expectation.
const START_LIMIT: Duration = Duration::from_secs(30);

/// A spawned server that is killed and reaped however the test leaves — return,
/// assertion failure, or panic.
struct Server {
    child: Child,
    pid: u32,
    /// The child's stdout a line at a time, read on a thread, so waiting for one
    /// line never blocks on a child that is still running.
    stdout: Receiver<String>,
    /// The lines already taken off `stdout`.
    seen: Vec<String>,
}

impl Server {
    /// Spawn the built server over `data_dir`, on a port the kernel picks
    /// (`--port 0`), with its output captured so a refusal's text can be read.
    fn spawn(data_dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pglet_server"))
            .args(["--port", "0"])
            .args(["--data-dir", &data_dir.to_string_lossy()])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn pglet_server");
        let pid = child.id();
        let out = child.stdout.take().expect("stdout is piped");
        let (tx, stdout) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            pid,
            stdout,
            seen: Vec::new(),
        }
    }

    /// Wait for the server's own report of the port it bound, and return it —
    /// `None` when it exits first or does not report within [`START_LIMIT`].
    ///
    /// The server binds only after it takes the data-dir lock, so its report proves
    /// that this child holds the lock and listens. A connect to a port picked in
    /// advance proves neither: whatever else answered on that port passed for the
    /// server, and a lock file read after it was missing (measured, once, under load).
    fn wait_listening(&mut self) -> Option<u16> {
        let deadline = Instant::now() + START_LIMIT;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let line = self.stdout.recv_timeout(left).ok()?;
            let port = line
                .strip_prefix("pglet listening on 127.0.0.1:")
                .and_then(|port| port.parse().ok());
            self.seen.push(line);
            if port.is_some() {
                return port;
            }
        }
    }

    /// Wait for it to exit, up to `limit`.
    ///
    /// Not a bare `wait()`: if the guard under test were removed, the refused
    /// server would come up and serve forever, and a bare `wait()` would hang
    /// the gate run instead of failing it. A vacuous guard has to be *loud*.
    fn wait_within(&mut self, limit: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => return None,
            }
        }
        None
    }

    /// Everything it wrote, both streams. Call after it has exited: the stdout
    /// thread ends, and `stderr` reaches its end, only when the child has.
    fn output(&mut self) -> String {
        self.seen.extend(self.stdout.iter());
        let mut out = self.seen.join("\n");
        if let Some(mut stderr) = self.child.stderr.take() {
            out.push('\n');
            let _ = stderr.read_to_string(&mut out);
        }
        out
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Spawn a server over `data_dir` and wait until it reports listening, then connect
/// to the port it reported, which proves it serves. Otherwise kill it and panic with
/// `failure` and everything it wrote — killed first, because its output only ends
/// when it does.
fn start(data_dir: &Path, failure: &str) -> Server {
    let mut server = Server::spawn(data_dir);
    let Some(port) = server.wait_listening() else {
        server.kill();
        panic!("{failure}: {}", server.output());
    };
    assert_ne!(
        port, 0,
        "the listening line names the bound port, not the requested one"
    );
    if let Err(e) = TcpStream::connect(("127.0.0.1", port)) {
        server.kill();
        panic!("{failure}: it reported port {port}, which refused: {e}");
    }
    server
}

fn lock_body(lock_path: &Path, what: &str) -> String {
    std::fs::read_to_string(lock_path)
        .unwrap_or_else(|e| panic!("read the lock file {what}: {e}"))
        .trim()
        .to_owned()
}

#[test]
fn a_second_server_over_one_data_dir_is_refused_naming_the_holder() {
    let dir = ScratchDir::new("data_dir_lock_refusal");

    let holder = start(&dir.path, "the first server never came up");

    let mut second = Server::spawn(&dir.path);
    let status = second
        .wait_within(Duration::from_secs(10))
        .expect("the second server must exit rather than serve the locked dir");
    let output = second.output();

    assert!(
        !status.success(),
        "the second server must refuse the locked dir, exited {status:?}: {output}"
    );
    assert!(
        output.contains("data dir locked"),
        "the refusal must say what went wrong: {output}"
    );
    assert!(
        output.contains(&format!("pid {}", holder.pid)),
        "the refusal must name the holder (pid {}): {output}",
        holder.pid
    );
}

#[test]
fn a_sigkilled_holder_leaves_a_lock_file_that_does_not_block_the_next_server() {
    let dir = ScratchDir::new("data_dir_lock_sigkill");
    let lock_path = dir.path.join("pglet.lock");

    let mut first = start(&dir.path, "the first server never came up");
    let first_pid = first.pid;

    assert_eq!(
        lock_body(&lock_path, "while held"),
        first_pid.to_string(),
        "the lock file body names the live holder"
    );

    // SIGKILL: no `Drop`, so nothing truncates the body and nothing unlinks the
    // file. Reap, so the descriptor is gone and the kernel has released the lock.
    first.kill();

    assert_eq!(
        lock_body(&lock_path, "after the kill"),
        first_pid.to_string(),
        "a killed holder leaves its pid behind — the body is informational, not the lock"
    );

    let second = start(
        &dir.path,
        "a new server must take over a killed holder's data dir",
    );

    assert_eq!(
        lock_body(&lock_path, "under the new holder"),
        second.pid.to_string(),
        "the new holder replaced the dead pid in the body"
    );
}
