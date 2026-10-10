//! Kernel-arbitrated exclusive file lock, over `std::fs::File::try_lock`.
//!
//! `try_lock` is `flock(2)` with `LOCK_EX | LOCK_NB` on Linux: an advisory lock
//! held on the *open file description*, which the kernel releases when the last
//! descriptor for it closes. Closing happens on process exit whatever ends the
//! process — a clean return, a panic, or SIGKILL — so a crashed holder leaves no
//! lock to recover and no pid-reuse ambiguity. That is the whole reason this is a
//! kernel lock and not a pidfile: being killed partway is the common case for
//! both of its callers (a `--data-dir` server and the gate runner), and a pidfile
//! would need stale-entry heuristics that are wrong exactly when they matter.
//!
//! The release on death is proved once, with real child processes, in
//! `crates/pglet_server/tests/data_dir_lock.rs`, and the gate runner inherits that proof
//! only because it takes this same implementation; a second copy would forfeit it.
//!
//! The file *body* holds the holder's pid in decimal, for the human reading the
//! refusal message and for `cat`. It is informational only — the locking is the
//! kernel's, so a hand-edited, empty, or stale body changes what a refusal can
//! *say*, never who holds the lock. A lock file left behind by a killed holder is
//! therefore not stale in any operative sense: the next acquirer takes the flock
//! and overwrites the body.
//!
//! ## Why the file is truncated on release rather than removed
//!
//! A flock lives on the inode, not the path. Unlinking on release opens a window
//! where one acquirer holds an fd to the original inode while a second finds the
//! path missing, creates a fresh inode, and locks that — two holders of "the
//! lock" on two inodes. [`FileLock`]'s `Drop` truncates the body and leaves the
//! directory entry, so every acquirer reaches the same inode and the kernel
//! arbitrates.
//!
//! Native-only, behind the `file_lock` feature, so the wasm build never compiles
//! `std::fs` on this path.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Why a lock could not be taken.
///
/// [`Self::Held`] is the expected outcome of a second concurrent run and reads as
/// a diagnosis; everything else is an I/O failure on the lock file itself.
#[derive(Debug, thiserror::Error)]
pub enum FileLockError {
    /// Another live process holds the lock. `pid` is the holder's, read from the
    /// file body — `None` when the body is empty or unparseable (the holder was
    /// raced between its `try_lock` and its write, or something hand-edited it).
    #[error("lock held by {}", display_pid(*pid))]
    Held { pid: Option<u32> },

    /// Opening, reading, writing, or truncating the lock file failed.
    #[error("file lock io error: {0}")]
    Io(#[source] std::io::Error),
}

/// An exclusive lock on a file, held for as long as this value lives.
///
/// Acquire with [`FileLock::acquire`]; release by dropping it (or by dying —
/// see the module docs). Callers hold it and never read it: the value *is* the
/// lock.
#[derive(Debug)]
pub struct FileLock {
    /// Dropping this closes the descriptor, which releases the kernel lock.
    file: File,
}

impl FileLock {
    /// Take an exclusive lock on `path`, creating the file if it does not exist,
    /// and write the current pid as the body.
    ///
    /// Returns [`FileLockError::Held`] immediately — never blocks — when another
    /// process holds it.
    ///
    /// # Errors
    ///
    /// [`FileLockError::Held`] when the lock is taken; [`FileLockError::Io`] when
    /// the parent directory cannot be created or the lock file cannot be opened,
    /// locked, truncated, or written.
    pub fn acquire(path: &Path) -> Result<Self, FileLockError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(FileLockError::Io)?;
        }

        // `truncate(false)`: a waiting acquirer must be able to read the current
        // holder's pid, so opening the file must not blank it. The truncate
        // happens below, after the lock is ours.
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(FileLockError::Io)?;

        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(FileLockError::Held {
                    pid: read_holder_pid(&mut file),
                });
            }
            Err(TryLockError::Error(e)) => return Err(FileLockError::Io(e)),
        }

        // Replace whatever a previous holder left. Seek first: the open left the
        // cursor at 0, but `read_holder_pid` is not the only thing that could
        // have moved it in a future edit, and a write at a stale offset would
        // leave a body that parses as the wrong pid.
        file.set_len(0).map_err(FileLockError::Io)?;
        file.seek(SeekFrom::Start(0)).map_err(FileLockError::Io)?;
        writeln!(file, "{}", std::process::id()).map_err(FileLockError::Io)?;

        Ok(Self { file })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Truncate while the lock is still ours, so a concurrent reader sees
        // "no holder" rather than the pid of a process that has released. Then
        // the `File` drops, closing the descriptor and releasing the kernel
        // lock. Do not unlink — see the module docs on the two-inode race.
        let _ = self.file.set_len(0);
    }
}

/// Read the holder pid out of a lock file body. Best effort by construction:
/// the body is informational, so an unreadable or unparseable one yields `None`
/// and the caller says "pid unknown" rather than inventing a pid.
fn read_holder_pid(file: &mut File) -> Option<u32> {
    let mut buf = String::new();
    file.read_to_string(&mut buf).ok()?;
    buf.trim().parse().ok()
}

/// Render an optional holder pid for a refusal message.
fn display_pid(pid: Option<u32>) -> String {
    pid.map_or_else(|| "pid unknown".to_owned(), |p| format!("pid {p}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// A fresh lock path per test, scoped to this process and removed when the
    /// test ends.
    ///
    /// Three parts, each load-bearing. The `remove_file` on the way in
    /// makes the path fresh — a lock test that inherits a previous run's file is
    /// testing the previous run. The pid makes it this process's — a fixed path
    /// in a shared temp directory is reachable by a second `cargo test -p
    /// pglet_core` (a hand run beside a gate run's `test:rust`, or two checkouts
    /// on one box), whose `remove_file` then unlinks the inode under a live
    /// holder and opens exactly the two-inode window the module docs describe.
    /// Measured: 1 of 6 concurrent pairs failed with `Held { pid: None }`, which
    /// reads like a lock bug and is not one. And the `Drop` takes the file away
    /// again, because a pid-unique path is a *new* path every run: without it
    /// every run leaves a file per test behind in the temp directory. It is a
    /// `Drop` rather than a line at the end of each test so a panicking test
    /// cleans up too.
    struct LockPath {
        path: PathBuf,
    }

    impl LockPath {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("{name}.{}.lock", std::process::id()));
            let _ = fs::remove_file(&path);
            Self { path }
        }
    }

    impl Drop for LockPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn drop_releases_the_lock() {
        let path = LockPath::new("pglet_file_lock_release");
        let lock = FileLock::acquire(&path.path).expect("first acquire");
        drop(lock);
        let _second = FileLock::acquire(&path.path).expect("re-acquire after drop");
    }

    #[test]
    fn a_second_acquire_is_refused_with_the_holder_pid() {
        let path = LockPath::new("pglet_file_lock_held");
        let _held = FileLock::acquire(&path.path).expect("first acquire");
        match FileLock::acquire(&path.path) {
            Err(FileLockError::Held { pid }) => {
                assert_eq!(pid, Some(std::process::id()), "holder pid is reported");
            }
            other => panic!("expected Held, got {other:?}"),
        }
    }

    #[test]
    fn release_empties_the_body_and_keeps_the_dirent() {
        let path = LockPath::new("pglet_file_lock_truncate");
        let lock = FileLock::acquire(&path.path).expect("acquire");
        let held = fs::read_to_string(&path.path).expect("read held body");
        assert_eq!(held.trim(), std::process::id().to_string());
        drop(lock);
        assert!(path.path.exists(), "the directory entry survives release");
        let released = fs::read_to_string(&path.path).expect("read released body");
        assert!(released.is_empty(), "released body is empty: {released:?}");
    }

    #[test]
    fn a_lock_file_left_by_a_dead_holder_is_taken_over() {
        // What a SIGKILLed holder leaves on disk: the file, with its pid still
        // in it, and no kernel lock. The pid is not consulted — the flock is —
        // so the acquire succeeds and the body is replaced.
        let path = LockPath::new("pglet_file_lock_stale");
        fs::write(&path.path, "999999999\n").expect("seed a dead holder's body");
        let _lock = FileLock::acquire(&path.path).expect("acquire over a dead holder's file");
        let body = fs::read_to_string(&path.path).expect("read body");
        assert_eq!(body.trim(), std::process::id().to_string());
    }

    #[test]
    fn an_unparseable_body_refuses_without_inventing_a_pid() {
        let path = LockPath::new("pglet_file_lock_garbage");
        let _held = FileLock::acquire(&path.path).expect("first acquire");
        fs::write(&path.path, "not a pid").expect("overwrite the body under the holder");
        match FileLock::acquire(&path.path) {
            Err(FileLockError::Held { pid: None }) => {}
            other => panic!("expected Held with no pid, got {other:?}"),
        }
    }
}
