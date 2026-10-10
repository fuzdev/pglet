//! Byte-level block I/O abstraction for single-file storage backends.
//!
//! `BlockIo` is the thin seam between a page store and the bytes it persists:
//! positioned reads/writes (`read_at`/`write_at`), an explicit `sync` (fsync),
//! and length management (`len`/`set_len`). It is deliberately minimal — the
//! same five operations are available natively over `std::fs::File`
//! (`pread`/`pwrite` via `std::os::unix::fs::FileExt`) and in the browser over
//! an OPFS `FileSystemSyncAccessHandle` (read-at/write-at/flush/truncate/
//! get-size). `DoubleMetaStore` is generic over this trait, so the same store
//! runs on both targets.
//!
//! The trait itself and `CountingBlockIo` are portable (no platform-specific
//! code). The native `File` shim is gated behind the `file_store` feature so it
//! never enters the wasm32 build.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Positioned, syncable byte I/O over a single backing store (a file).
///
/// All offsets are absolute byte offsets from the start of the store.
/// Implementations must be safe to share across threads (`Send + Sync`); the
/// native `File` shim relies on positioned reads/writes that don't disturb a
/// shared seek cursor.
pub trait BlockIo: Send + Sync {
    /// Read into `buf` starting at `offset`, returning the number of bytes read.
    ///
    /// A short read (fewer bytes than `buf.len()`) is returned as-is and means
    /// end-of-store was reached; callers that require a full read must check.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the underlying read fails.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize>;

    /// Write all of `buf` starting at `offset`.
    ///
    /// Implementations must write the entire buffer (the native shim uses
    /// `write_all_at`); a partial write is reported as an error.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the underlying write fails or is short.
    fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()>;

    /// Flush buffered writes to durable storage (fsync).
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the sync fails.
    fn sync(&self) -> std::io::Result<()>;

    /// Current length of the store in bytes.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the length cannot be determined.
    fn len(&self) -> std::io::Result<u64>;

    /// Set the length of the store, extending (zero-filled) or truncating.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the resize fails.
    fn set_len(&self, size: u64) -> std::io::Result<()>;

    /// Whether the store is currently empty (zero length).
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the length cannot be determined.
    fn is_empty(&self) -> std::io::Result<bool> {
        Ok(self.len()? == 0)
    }
}

/// Native `std::fs::File`-backed `BlockIo`.
///
/// Uses positioned reads/writes (`pread`/`pwrite` via `FileExt`) so concurrent
/// access doesn't fight over a seek cursor, and `sync_all` for fsync. Gated
/// behind the `file_store` feature: the unix `FileExt` path must never compile
/// into the wasm32 build.
#[cfg(feature = "file_store")]
#[derive(Debug)]
pub struct FileBlockIo {
    file: std::fs::File,
}

#[cfg(feature = "file_store")]
impl FileBlockIo {
    /// Open (creating if absent) the file at `path` for read+write block I/O.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file cannot be opened or created.
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        Ok(Self { file })
    }

    /// Wrap an already-open file as a `BlockIo`.
    pub const fn from_file(file: std::fs::File) -> Self {
        Self { file }
    }
}

#[cfg(feature = "file_store")]
impl BlockIo for FileBlockIo {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::os::unix::fs::FileExt;
        self.file.read_at(buf, offset)
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
        use std::os::unix::fs::FileExt;
        self.file.write_all_at(buf, offset)
    }

    fn sync(&self) -> std::io::Result<()> {
        self.file.sync_all()
    }

    fn len(&self) -> std::io::Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    fn set_len(&self, size: u64) -> std::io::Result<()> {
        self.file.set_len(size)
    }
}

/// Snapshot of the I/O activity observed by a [`CountingBlockIo`].
///
/// All counters are cumulative since the wrapper was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockIoStats {
    /// Number of `sync` (fsync) calls.
    pub syncs: u64,
    /// Number of `write_at` calls.
    pub writes: u64,
    /// Total bytes passed to `write_at`.
    pub bytes_written: u64,
    /// Number of `read_at` calls.
    pub reads: u64,
    /// Total bytes returned by `read_at`.
    pub bytes_read: u64,
    /// Cumulative nanoseconds spent inside the inner `write_at` calls.
    pub write_nanos: u64,
    /// Cumulative nanoseconds spent inside the inner `sync` (fsync) calls.
    pub sync_nanos: u64,
}

/// The atomic counters shared between a [`CountingBlockIo`] and any
/// [`CountingHandle`]s cloned from it.
#[derive(Debug, Default)]
struct CountingInner {
    syncs: AtomicU64,
    writes: AtomicU64,
    bytes_written: AtomicU64,
    reads: AtomicU64,
    bytes_read: AtomicU64,
    write_nanos: AtomicU64,
    sync_nanos: AtomicU64,
}

impl CountingInner {
    fn stats(&self) -> BlockIoStats {
        BlockIoStats {
            syncs: self.syncs.load(Ordering::Relaxed),
            writes: self.writes.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            reads: self.reads.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            write_nanos: self.write_nanos.load(Ordering::Relaxed),
            sync_nanos: self.sync_nanos.load(Ordering::Relaxed),
        }
    }

    fn reset(&self) {
        self.syncs.store(0, Ordering::Relaxed);
        self.writes.store(0, Ordering::Relaxed);
        self.bytes_written.store(0, Ordering::Relaxed);
        self.reads.store(0, Ordering::Relaxed);
        self.bytes_read.store(0, Ordering::Relaxed);
        self.write_nanos.store(0, Ordering::Relaxed);
        self.sync_nanos.store(0, Ordering::Relaxed);
    }
}

/// A cloneable reader over a [`CountingBlockIo`]'s counters.
///
/// `CountingBlockIo` is moved into a store as a `Box<dyn BlockIo>`, after which
/// `.stats()` can no longer be called on it directly. Clone a `CountingHandle`
/// *before* boxing the counter to keep an out-of-band view of the same atomics,
/// so a benchmark can read fsync/write/byte counts around each commit.
#[derive(Debug, Clone)]
pub struct CountingHandle {
    inner: Arc<CountingInner>,
}

impl CountingHandle {
    /// Current cumulative I/O statistics.
    pub fn stats(&self) -> BlockIoStats {
        self.inner.stats()
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.inner.reset();
    }
}

/// A pass-through `BlockIo` decorator that counts fsyncs, reads, and writes.
///
/// Wraps any `BlockIo` and records call counts and byte volumes via atomics,
/// so a benchmark can assert how many fsyncs and page writes a commit performs
/// without touching the underlying store's behavior. The counters live behind
/// an `Arc` so a cloneable [`CountingHandle`] can read them after this wrapper
/// has been boxed into a store (see `handle`). Dependency-free.
pub struct CountingBlockIo<B: BlockIo> {
    inner: B,
    counters: Arc<CountingInner>,
}

impl<B: BlockIo> std::fmt::Debug for CountingBlockIo<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CountingBlockIo")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl<B: BlockIo> CountingBlockIo<B> {
    /// Wrap `inner`, starting all counters at zero.
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            counters: Arc::new(CountingInner::default()),
        }
    }

    /// A cloneable reader over this wrapper's counters.
    ///
    /// Clone the handle before moving the wrapper into a store as a
    /// `Box<dyn BlockIo>`; the handle then continues to observe the same
    /// atomics the boxed wrapper updates.
    pub fn handle(&self) -> CountingHandle {
        CountingHandle {
            inner: Arc::clone(&self.counters),
        }
    }

    /// Current cumulative I/O statistics.
    pub fn stats(&self) -> BlockIoStats {
        self.counters.stats()
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        self.counters.reset();
    }

    /// Borrow the wrapped `BlockIo`.
    pub const fn inner(&self) -> &B {
        &self.inner
    }
}

impl<B: BlockIo> BlockIo for CountingBlockIo<B> {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read_at(offset, buf)?;
        self.counters.reads.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes_read
            .fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
        let start = std::time::Instant::now();
        let result = self.inner.write_at(offset, buf);
        let elapsed = start.elapsed().as_nanos() as u64;
        self.counters
            .write_nanos
            .fetch_add(elapsed, Ordering::Relaxed);
        result?;
        self.counters.writes.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes_written
            .fetch_add(buf.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    fn sync(&self) -> std::io::Result<()> {
        let start = std::time::Instant::now();
        let result = self.inner.sync();
        let elapsed = start.elapsed().as_nanos() as u64;
        self.counters
            .sync_nanos
            .fetch_add(elapsed, Ordering::Relaxed);
        result?;
        self.counters.syncs.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn len(&self) -> std::io::Result<u64> {
        self.inner.len()
    }

    fn set_len(&self, size: u64) -> std::io::Result<()> {
        self.inner.set_len(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btree::mem_block_io::MemBlockIo;

    #[test]
    fn counting_tracks_reads_writes_syncs() {
        let io = CountingBlockIo::new(MemBlockIo::new());

        io.write_at(0, &[1, 2, 3, 4]).unwrap();
        io.write_at(4, &[5, 6]).unwrap();
        io.sync().unwrap();

        let mut buf = [0u8; 4];
        let n = io.read_at(0, &mut buf).unwrap();

        let stats = io.stats();
        assert_eq!(stats.writes, 2);
        assert_eq!(stats.bytes_written, 6);
        assert_eq!(stats.syncs, 1);
        assert_eq!(stats.reads, 1);
        assert_eq!(stats.bytes_read, 4);
        assert_eq!(n, 4);
        assert_eq!(buf, [1, 2, 3, 4]);
    }

    #[test]
    fn read_past_end_returns_short() {
        let io = MemBlockIo::new();
        io.write_at(0, &[1, 2, 3]).unwrap();

        let mut buf = [0u8; 8];
        let n = io.read_at(0, &mut buf).unwrap();
        assert_eq!(n, 3);

        // Reading entirely past the end returns 0.
        let n = io.read_at(100, &mut buf).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn reset_zeroes_counters() {
        let io = CountingBlockIo::new(MemBlockIo::new());
        io.write_at(0, &[1]).unwrap();
        io.sync().unwrap();
        io.reset();
        assert_eq!(io.stats(), BlockIoStats::default());
    }

    #[test]
    fn handle_observes_counters_after_boxing() {
        let io = CountingBlockIo::new(MemBlockIo::new());
        let handle = io.handle();

        // Box up the wrapper as a trait object (mirrors moving it into a store);
        // the cloned handle must still see the writes/syncs it performs.
        let boxed: Box<dyn BlockIo> = Box::new(io);
        boxed.write_at(0, &[1, 2, 3]).unwrap();
        boxed.sync().unwrap();

        let stats = handle.stats();
        assert_eq!(stats.writes, 1);
        assert_eq!(stats.bytes_written, 3);
        assert_eq!(stats.syncs, 1);

        // Reset via the handle is visible too.
        handle.reset();
        assert_eq!(handle.stats(), BlockIoStats::default());
    }
}
