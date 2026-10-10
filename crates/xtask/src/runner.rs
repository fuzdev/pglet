//! Shared plumbing for the task runners — [`crate::gates`] and [`crate::measure`].
//!
//! Both drive a sequence of subprocesses and both need the same four properties,
//! which is the whole reason this module exists rather than each rolling its own:
//!
//! 1. **Exclusivity.** Two concurrent runs fight over the same spawned servers,
//!    the same scratch Postgres databases and the same box, and the damage is not
//!    always loud — interleaved runs can produce plausible-looking failures, or
//!    (for a measurement) plausible-looking *numbers*. `flock` and not a pidfile:
//!    being killed partway is the common case, and the kernel releases a `flock`
//!    on process exit, SIGKILL included, so there is no stale lock to clear.
//! 2. **Exit codes.** A killed task's log reads green right up to the kill, so the
//!    tail of the output is never evidence that it finished. Every task's status
//!    is captured, with a signal death distinguished from a non-zero exit.
//! 3. **Logs.** Per-task, on disk, under `target/<runner>/`, so a run's output
//!    survives the scrollback it would otherwise live in.
//! 4. **Timing.** Per task, reported in the summary.
//!
//! The runners differ in what they do with a result — gates extracts counters and
//! compares a baseline, measure prints tables and cleans up a scratch database —
//! and in whether they stream (see `tee` on [`run_logged`]).

use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use pglet_core::file_lock::{FileLock, FileLockError};

/// The workspace root, resolved from this crate's manifest directory at compile
/// time so a command behaves the same however it is invoked.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// A runner's log directory, held under an exclusive lock.
///
/// The lock lives as long as this value; dropping it (or dying) releases it.
pub struct RunDir {
    pub log_dir: PathBuf,
    _lock: FileLock,
}

/// Create `target/<name>/` and take the exclusive lock over it.
///
/// `what` names the thing being serialized, for the "already in progress"
/// message — the reader of that message needs to know what they are colliding
/// with, not merely that they are.
///
/// # Errors
///
/// The directory cannot be created, the lock is held by another run, or the
/// lock cannot be acquired. The message is caller-printable as-is.
pub fn lock_run_dir(root: &Path, name: &str, what: &str) -> Result<RunDir, String> {
    let log_dir = root.join("target").join(name);
    std::fs::create_dir_all(&log_dir)
        .map_err(|e| format!("cannot create {}: {e}", log_dir.display()))?;
    match FileLock::acquire(&log_dir.join("run.lock")) {
        Ok(lock) => Ok(RunDir {
            log_dir,
            _lock: lock,
        }),
        Err(FileLockError::Held { pid }) => {
            let who = pid.map_or_else(String::new, |p| format!(" (pid {p})"));
            Err(format!(
                "a {what} is already in progress{who}\n\
                 note: {what}s are exclusive — two at once corrupt each other's results"
            ))
        }
        Err(e) => Err(format!("cannot acquire the {what} lock: {e}")),
    }
}

/// What one subprocess did.
#[derive(Debug)]
pub struct TaskRun {
    /// `None` when the child died on a signal or could not be spawned.
    pub exit: Option<i32>,
    /// The signal that killed the child, when one did.
    pub signal: Option<i32>,
    pub secs: f64,
    pub log: PathBuf,
    /// The child's combined output, also written to [`Self::log`].
    pub output: String,
}

impl TaskRun {
    #[must_use]
    pub const fn ok(&self) -> bool {
        matches!(self.exit, Some(0))
    }

    /// A short status for a summary line.
    ///
    /// `KILLED` names the signal when there is one: "killed" and "killed by the
    /// OOM killer" are different diagnoses, and on a long measurement run the
    /// difference is the whole message.
    #[must_use]
    pub fn status(&self) -> String {
        match (self.exit, self.signal) {
            (Some(0), _) => "ok".to_owned(),
            (Some(code), _) => format!("FAIL({code})"),
            (None, Some(sig)) => format!("KILLED(sig {sig})"),
            (None, None) => "KILLED".to_owned(),
        }
    }
}

/// A `deno task <task>` command, the shape most tasks in this workspace take.
#[must_use]
pub fn deno_task(task: &str) -> Command {
    let mut cmd = Command::new("deno");
    cmd.args(["task", task]);
    cmd
}

/// Run one subprocess to completion, capturing its combined output to `log`.
///
/// With `tee`, the output is also written through to this process's stderr as it
/// arrives. That is not a cosmetic choice per runner: a gate run prints its own
/// per-task progress and is read from the summary afterwards, while a
/// measurement run's *tables are the product* and a silent multi-minute step is
/// unreadable.
///
/// `LD_LIBRARY_PATH` is dropped from the child's environment. `cargo run -p
/// xtask` sets it so *this* binary finds its dylibs, and it would otherwise be
/// inherited by every task. Deno refuses to spawn a subprocess under a scoped
/// `--allow-run` when it is set (the variable can redirect which libraries load,
/// so a narrow allowlist would not mean what it says), which made `size:check` —
/// the one gate that shells out, to `gzip` — fail under the runner while passing
/// when run by hand. Cargo re-derives it for the binaries it runs, so dropping
/// the inherited one costs nothing and keeps the runner's environment out of the
/// tasks it runs.
pub fn run_logged(cmd: &mut Command, root: &Path, log: PathBuf, tee: bool) -> TaskRun {
    let started = Instant::now();
    let spawned = cmd
        .current_dir(root)
        .env_remove("LD_LIBRARY_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let (exit, signal, combined) = match spawned {
        Ok(mut child) => {
            let buf = Arc::new(Mutex::new(Vec::new()));
            let pumps: Vec<_> = [
                child.stdout.take().map(PipeSource::Out),
                child.stderr.take().map(PipeSource::Err),
            ]
            .into_iter()
            .flatten()
            .map(|src| spawn_pump(src, Arc::clone(&buf), tee))
            .collect();
            let status = child.wait();
            for pump in pumps {
                let _ = pump.join();
            }
            let bytes = buf.lock().map(|b| b.clone()).unwrap_or_default();
            match status {
                Ok(status) => (status.code(), signal_of(status), bytes),
                Err(e) => (
                    None,
                    None,
                    [bytes, format!("wait failed: {e}\n").into()].concat(),
                ),
            }
        }
        Err(e) => (
            None,
            None,
            format!("failed to spawn `{}`: {e}\n", cmd.get_program().display()).into_bytes(),
        ),
    };

    let _ = std::fs::write(&log, &combined);
    TaskRun {
        exit,
        signal,
        secs: started.elapsed().as_secs_f64(),
        log,
        output: String::from_utf8_lossy(&combined).into_owned(),
    }
}

enum PipeSource {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

/// Drain one pipe into the shared buffer, optionally mirroring it to stderr.
///
/// Chunk-level rather than line-level so a progress line written without a
/// trailing newline (`  [rep 1/2] pglite … `) appears when it is written rather
/// than when the line it belongs to finally ends.
fn spawn_pump(src: PipeSource, buf: Arc<Mutex<Vec<u8>>>, tee: bool) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader: Box<dyn Read> = match src {
            PipeSource::Out(r) => Box::new(r),
            PipeSource::Err(r) => Box::new(r),
        };
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tee {
                        let mut err = std::io::stderr();
                        let _ = err.write_all(&chunk[..n]);
                        let _ = err.flush();
                    }
                    if let Ok(mut b) = buf.lock() {
                        b.extend_from_slice(&chunk[..n]);
                    }
                }
            }
        }
    })
}

#[cfg(unix)]
fn signal_of(status: std::process::ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(&status)
}

#[cfg(not(unix))]
const fn signal_of(_status: std::process::ExitStatus) -> Option<i32> {
    None
}

/// Parse a comma-separated CLI list, validating every name against `known`.
///
/// `noun` names what the list holds ("gate", "step"), for the error message.
///
/// An unknown name is an error rather than a silent no-op, the same rule the
/// fixture harness applies to an unknown `-- @directive`: a typo must change the
/// run's outcome loudly, never quietly measure or gate a different set than the
/// one asked for.
///
/// # Errors
///
/// Any name not in `known`, listing the valid ones.
pub fn parse_name_list(
    raw: &str,
    known: &[&str],
    flag: &str,
    noun: &str,
) -> Result<Vec<String>, String> {
    let names: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    let unknown: Vec<&str> = names
        .iter()
        .filter(|n| !known.contains(&n.as_str()))
        .map(String::as_str)
        .collect();
    if unknown.is_empty() {
        Ok(names)
    } else {
        Err(format!(
            "`{flag}` names unknown {noun}(s): {}\n       known: {}",
            unknown.join(", "),
            known.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_name_list;

    #[test]
    fn a_known_list_parses_and_trims() {
        let got = parse_name_list(" a, b ,", &["a", "b"], "--only", "step").expect("parses");
        assert_eq!(got, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn an_unknown_name_errors_and_names_both_sides() {
        let err = parse_name_list("a,zzz", &["a", "b"], "--only", "step").expect_err("rejects");
        assert!(err.contains("zzz"), "names the typo: {err}");
        assert!(err.contains("a, b"), "names the valid set: {err}");
    }

    #[test]
    fn an_empty_list_is_empty_rather_than_one_empty_name() {
        assert!(
            parse_name_list("", &["a"], "--only", "step")
                .expect("parses")
                .is_empty()
        );
    }
}

#[cfg(test)]
mod lock_tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::{PathBuf, lock_run_dir};

    /// A fresh root per test — a lock test that inherits a previous run's
    /// directory is testing the previous run — removed again when the test ends.
    ///
    /// The pid in the name keeps two concurrent `cargo test -p xtask` runs off
    /// each other's directory; it also makes the path a new one every run, so
    /// the removal has to happen on the way out as well. A `Drop` rather than a
    /// line at the end of each test, so a panicking test cleans up too.
    struct ScratchRoot {
        path: PathBuf,
    }

    impl ScratchRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "pglet_runner_lock_test_{tag}_{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            Self { path }
        }
    }

    impl Drop for ScratchRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn a_second_run_is_refused_naming_the_holder_and_what_it_collides_with() {
        let root = ScratchRoot::new("held");
        let held = lock_run_dir(&root.path, "gates", "gate run").expect("first run takes the lock");
        let refusal = lock_run_dir(&root.path, "gates", "gate run")
            .err()
            .expect("a second run must be refused");
        assert!(
            refusal.contains("a gate run is already in progress"),
            "the message names what is colliding: {refusal}"
        );
        assert!(
            refusal.contains(&format!("pid {}", std::process::id())),
            "the message names the holder: {refusal}"
        );
        drop(held);
    }

    #[test]
    fn releasing_the_lock_lets_the_next_run_in() {
        let root = ScratchRoot::new("release");
        let first =
            lock_run_dir(&root.path, "gates", "gate run").expect("first run takes the lock");
        drop(first);
        let _second = lock_run_dir(&root.path, "gates", "gate run").expect("the next run takes it");
    }

    #[test]
    fn two_runners_do_not_share_a_lock() {
        // `gates` and `measure` serialize against themselves, not against each
        // other: they lock `target/gates` and `target/release_measurements`.
        let root = ScratchRoot::new("distinct");
        let _gates = lock_run_dir(&root.path, "gates", "gate run").expect("gates lock");
        let _measure = lock_run_dir(&root.path, "release_measurements", "measurement run")
            .expect("measure takes its own lock");
    }
}
