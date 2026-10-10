//! In-memory `BlockIo` doubles for pglet's storage/durability tests.
//!
//! [`MemBlockIo`] is one drop-surviving in-memory block device serving three test roles:
//!
//! - a **plain** in-memory store (unit tests),
//! - a **reopen-shareable** store (durability/recovery tests — [`MemBlockIo::reopen`]
//!   hands back a second handle over the *same* bytes, simulating reopening a file
//!   after a crash), and
//! - a **fault-injecting** store (crash tests — arm a [`FaultPolicy`] to break one
//!   of the steps of a commit).
//!
//! # The commit byte-sequence the fault policy targets
//!
//! A no-WAL double-meta commit is, in order: data-page `write_at`s (offset ≥
//! [`DATA_REGION_START`]) → fsync #1 → the save's retired-log and free-list chain-page
//! `write_at`s (data region too) → the alternate meta-slot `write_at` (offset
//! < [`DATA_REGION_START`]) → fsync #2 (**the atomic commit point**) → an
//! in-memory active-slot flip. The slot modes gate on the write *offset*: the meta
//! write is the only `write_at` below [`DATA_REGION_START`].
//!
//! Three of the [`FaultMode`]s break the meta write, the last write of a commit (its
//! fsync follows). [`FaultMode::FailDataWritesAfter`] and
//! [`FaultMode::FailSyncs`] reach steps ahead of it — a data-region write and a
//! `sync` — which is what a caller asking "does a batch that fails part-way leave
//! anything readable?" needs: the first sync a commit reaches is the data fsync
//! inside `apply_batch`, before the root is published. `FailSyncs` is not scoped to
//! that one, though — it fails **every** sync, the meta fsync included.
//! [`FaultMode::FailSyncsAfter`] counts syncs instead, so a caller who knows a commit's
//! sequence can fail the meta fsync alone, after the slot's bytes are in the buffer —
//! a failed writeback that leaves them readable.
//!
//! Reads are faithful unless [`FaultPolicy::arm_fail_data_reads`] asks for the next
//! few data-region reads to fail, independently of the mode. [`FaultPolicy::arm_before_data_read`]
//! is a seam rather than a fault: it runs a caller's closure on the reading thread just
//! before one data-region read copies its bytes, so a test can change what the store
//! holds between a reader's checks and its read, in a fixed order and with no second
//! thread.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pglet_storage::btree::BlockIo;

/// Offset where data pages begin (`META_SLOT_SIZE` 64 KiB × `META_SLOT_COUNT` 2).
///
/// Mirrors `double_meta.rs`'s private `DATA_REGION_START`: meta slots live below
/// this, data pages at or above it. Kept in sync with the engine layout by the
/// `data_region_start_matches_layout` test in `crates/pglet`.
pub const DATA_REGION_START: u64 = 64 * 1024 * 2;

/// What the [`FaultPolicy`] does to a commit's writes and syncs.
///
/// Each mode names its target: a meta-slot write (offset < [`DATA_REGION_START`]), a
/// data-region write, or a `sync`. Reads are faithful unless
/// [`FaultPolicy::arm_fail_data_reads`] says otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultMode {
    /// Faithful: every write hits the buffer in full, every sync is honored.
    None,
    /// Model "the meta write and its fsync never reached disk": drop every
    /// meta-slot `write_at` and turn `sync` into a no-op. The data pages and
    /// their fsync, written earlier in the commit, are untouched.
    DropSlotWrites,
    /// Model a torn/partial meta write from power loss: the *next* meta-slot
    /// `write_at` lands only its first `prefix` bytes, then returns `Ok`
    /// (deliberately violating the write-all contract — that *is* the power-loss
    /// condition). One-shot: after firing once it reverts to pass-through.
    TearNextSlotWrite { prefix: usize },
    /// Model a runtime **I/O error** (`ENOSPC`/`EIO`) on the durable commit point:
    /// every meta-slot `write_at` returns `Err` (writing nothing). Distinct from
    /// `DropSlotWrites`, which simulates a *crash* (the write call returns `Ok`,
    /// the bytes just never reach disk): here the caller *sees* the failure and
    /// must surface it, so it exercises the fail-loud (not the crash-recovery) path.
    FailSlotWrites,
    /// Model a runtime I/O error **part-way through a batch's page writes**: the
    /// first `successes` data-region `write_at`s land faithfully, every later one
    /// returns `Err`. The three slot modes target the meta slot; this one lands on
    /// data-region writes — `CowBtree::apply_batch`'s `store.put` loop, which runs before
    /// the data fsync and before the root is published, and past those the save's
    /// chain-page writes. Meta writes stay faithful under it.
    FailDataWritesAfter {
        /// How many data-region writes land before the failures start.
        successes: usize,
    },
    /// Model a runtime I/O error on **every `sync`** — the other half of an
    /// `apply_batch` failure. The first sync a commit reaches is the data fsync inside
    /// `apply_batch`, after every page write and before the root is published, so a
    /// caller that arms this over one statement gets that one; the mode itself does not
    /// distinguish the two fsyncs.
    FailSyncs,
    /// Model a failed writeback: the first `successes` syncs are honored and every later
    /// one returns an I/O error, the bytes written before it staying in the buffer, as a
    /// page cache holds them after a failed `fsync`. Counted from arming, so it can
    /// single out one fsync of a commit — the meta fsync, with the slot already written.
    FailSyncsAfter {
        /// How many syncs are honored before the failures start.
        successes: usize,
    },
}

/// What [`FaultPolicy::arm_before_data_read`] runs, handed the read's offset.
pub type DataReadHook = Box<dyn FnOnce(u64) + Send>;

/// The armed [`DataReadHook`] and how many data-region reads it lets pass first.
struct DataReadSeam(Mutex<Option<(usize, DataReadHook)>>);

impl std::fmt::Debug for DataReadSeam {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let armed = self.0.lock().is_ok_and(|slot| slot.is_some());
        f.debug_struct("DataReadSeam")
            .field("armed", &armed)
            .finish()
    }
}

impl DataReadSeam {
    /// The hook, taken, when this read is the one it waits for; otherwise one fewer read
    /// to let pass.
    fn take_for_read(&self) -> Option<DataReadHook> {
        let mut slot = self.0.lock().unwrap();
        match slot.as_mut() {
            Some((0, _)) => slot.take().map(|(_, hook)| hook),
            Some((skip, _)) => {
                *skip -= 1;
                None
            }
            None => None,
        }
    }
}

/// Arc-shared fault policy, armed/disarmed *after* the [`MemBlockIo`] is boxed.
///
/// Interior-mutable (atomics, plus the read seam's mutex, which every data-region read
/// takes) so the test keeps a handle to the same policy the boxed I/O consults on every
/// write, sync and data-region read.
#[derive(Debug)]
pub struct FaultPolicy {
    /// `0` = None, `1` = DropSlotWrites, `2` = TearNextSlotWrite,
    /// `3` = FailSlotWrites, `4` = FailDataWritesAfter, `5` = FailSyncs,
    /// `6` = FailSyncsAfter.
    mode: AtomicUsize,
    /// Prefix length for `TearNextSlotWrite`.
    tear_prefix: AtomicU64,
    /// Whether a one-shot tear has already fired.
    tear_fired: AtomicUsize,
    /// How many data-region writes `FailDataWritesAfter` lets land.
    data_write_budget: AtomicUsize,
    /// How many data-region writes have been seen since the mode was armed.
    data_writes_seen: AtomicUsize,
    /// Whether `FailDataWritesAfter` disarms itself once it has refused one write.
    data_write_once: AtomicUsize,
    /// Whether an injected failure is the device out of space (`ENOSPC`,
    /// `ErrorKind::StorageFull`) rather than a plain I/O error.
    disk_full: AtomicUsize,
    /// How many syncs `FailSyncsAfter` honors.
    sync_budget: AtomicUsize,
    /// How many syncs have been seen since `FailSyncsAfter` was armed.
    syncs_seen: AtomicUsize,
    /// How many of the next data-region reads fail.
    failing_data_reads: AtomicUsize,
    /// The closure one data-region read runs before it copies its bytes.
    before_data_read: DataReadSeam,
}

impl FaultPolicy {
    /// A disarmed policy (faithful I/O).
    pub const fn new() -> Self {
        Self {
            mode: AtomicUsize::new(0),
            tear_prefix: AtomicU64::new(0),
            tear_fired: AtomicUsize::new(0),
            data_write_budget: AtomicUsize::new(0),
            data_writes_seen: AtomicUsize::new(0),
            data_write_once: AtomicUsize::new(0),
            disk_full: AtomicUsize::new(0),
            sync_budget: AtomicUsize::new(0),
            syncs_seen: AtomicUsize::new(0),
            failing_data_reads: AtomicUsize::new(0),
            before_data_read: DataReadSeam(Mutex::new(None)),
        }
    }

    /// Arm "drop every meta-slot write + no-op the meta sync".
    pub fn arm_drop_slot_writes(&self) {
        self.mode.store(1, Ordering::SeqCst);
    }

    /// Arm a one-shot torn write of the next meta-slot `write_at` to `prefix` bytes.
    pub fn arm_tear_next_slot_write(&self, prefix: usize) {
        self.tear_prefix.store(prefix as u64, Ordering::SeqCst);
        self.tear_fired.store(0, Ordering::SeqCst);
        self.mode.store(2, Ordering::SeqCst);
    }

    /// Arm "every meta-slot write returns an I/O error" — the durable commit point
    /// fails loud (vs `arm_drop_slot_writes`, which fails silently like a crash).
    pub fn arm_fail_slot_writes(&self) {
        self.mode.store(3, Ordering::SeqCst);
    }

    /// Arm "the first `successes` data-region writes land, then every one fails" —
    /// an I/O error part-way through a batch's page writes.
    pub fn arm_fail_data_writes_after(&self, successes: usize) {
        self.disk_full.store(0, Ordering::SeqCst);
        self.data_write_once.store(0, Ordering::SeqCst);
        self.data_write_budget.store(successes, Ordering::SeqCst);
        self.data_writes_seen.store(0, Ordering::SeqCst);
        self.mode.store(4, Ordering::SeqCst);
    }

    /// [`Self::arm_fail_data_writes_after`], one-shot: the write after the first
    /// `successes` fails and the policy then returns to faithful pass-through — a fault
    /// that does not persist, so whatever the engine writes next lands.
    pub fn arm_fail_one_data_write_after(&self, successes: usize) {
        self.arm_fail_data_writes_after(successes);
        self.data_write_once.store(1, Ordering::SeqCst);
    }

    /// [`Self::arm_fail_data_writes_after`] with the device out of space: each failing
    /// write is `ENOSPC` (`ErrorKind::StorageFull`) — a full disk the file cannot grow
    /// on.
    pub fn arm_fill_disk_after(&self, successes: usize) {
        self.arm_fail_data_writes_after(successes);
        self.disk_full.store(1, Ordering::SeqCst);
    }

    /// The error an injected failure returns: `ENOSPC` once [`Self::arm_fill_disk_after`]
    /// armed it, a plain I/O error otherwise.
    fn injected(&self, what: &str) -> std::io::Error {
        if self.disk_full.load(Ordering::SeqCst) == 1 {
            std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                format!("injected {what}: no space left on device"),
            )
        } else {
            std::io::Error::other(format!("injected {what}"))
        }
    }

    /// Arm "every `sync` returns an I/O error" — the fsync half of the same failure.
    pub fn arm_fail_syncs(&self) {
        self.disk_full.store(0, Ordering::SeqCst);
        self.mode.store(5, Ordering::SeqCst);
    }

    /// Arm "the first `successes` syncs are honored, every later one fails" — with the
    /// bytes already written left in the buffer ([`FaultMode::FailSyncsAfter`]).
    pub fn arm_fail_syncs_after(&self, successes: usize) {
        self.disk_full.store(0, Ordering::SeqCst);
        self.sync_budget.store(successes, Ordering::SeqCst);
        self.syncs_seen.store(0, Ordering::SeqCst);
        self.mode.store(6, Ordering::SeqCst);
    }

    /// Fail the next `count` reads at or above [`DATA_REGION_START`] with an I/O error,
    /// whatever the mode; the meta slots stay readable.
    pub fn arm_fail_data_reads(&self, count: usize) {
        self.failing_data_reads.store(count, Ordering::SeqCst);
    }

    /// How many of the reads [`Self::arm_fail_data_reads`] asked to fail have not yet.
    pub fn failing_data_reads_left(&self) -> usize {
        self.failing_data_reads.load(Ordering::SeqCst)
    }

    /// Run `hook` on the reading thread, handed the read's offset, just before the data-region
    /// read that follows `skip` others copies its bytes — once. The backing bytes are not
    /// locked while it runs, so it may write them through another handle
    /// ([`MemBlockIo::reopen`]); the read then copies what the hook left.
    pub fn arm_before_data_read(&self, skip: usize, hook: DataReadHook) {
        *self.before_data_read.0.lock().unwrap() = Some((skip, hook));
    }

    /// Whether the hook [`Self::arm_before_data_read`] armed is still waiting for its read.
    pub fn before_data_read_armed(&self) -> bool {
        self.before_data_read.0.lock().unwrap().is_some()
    }

    /// Return to faithful pass-through, so the test can read back what the armed
    /// window left behind.
    pub fn disarm(&self) {
        self.mode.store(0, Ordering::SeqCst);
        self.failing_data_reads.store(0, Ordering::SeqCst);
        *self.before_data_read.0.lock().unwrap() = None;
    }

    fn current(&self) -> FaultMode {
        match self.mode.load(Ordering::SeqCst) {
            1 => FaultMode::DropSlotWrites,
            2 => FaultMode::TearNextSlotWrite {
                prefix: self.tear_prefix.load(Ordering::SeqCst) as usize,
            },
            3 => FaultMode::FailSlotWrites,
            4 => FaultMode::FailDataWritesAfter {
                successes: self.data_write_budget.load(Ordering::SeqCst),
            },
            5 => FaultMode::FailSyncs,
            6 => FaultMode::FailSyncsAfter {
                successes: self.sync_budget.load(Ordering::SeqCst),
            },
            _ => FaultMode::None,
        }
    }
}

impl Default for FaultPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// In-memory block device over a drop-surviving shared byte buffer, with optional
/// fault injection.
///
/// The buffer is an `Arc<Mutex<Vec<u8>>>`, so [`reopen`](Self::reopen) (and
/// [`Clone`]) see the exact bytes a dropped store left behind — the model for
/// reopening a file across a crash. `reopen` hands back a handle with a *fresh*
/// (disarmed) policy, so recovery I/O is always faithful; [`Clone`] keeps sharing
/// the same policy (use it to hand the boxed store a handle you can still arm).
#[derive(Debug, Clone)]
pub struct MemBlockIo {
    data: Arc<Mutex<Vec<u8>>>,
    policy: Arc<FaultPolicy>,
}

impl MemBlockIo {
    /// An empty in-memory store with a disarmed fault policy.
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(Vec::new())),
            policy: Arc::new(FaultPolicy::new()),
        }
    }

    /// An in-memory store pre-populated with `bytes` (e.g. a corrupted image in a
    /// fuzz iteration), disarmed.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            data: Arc::new(Mutex::new(bytes)),
            policy: Arc::new(FaultPolicy::new()),
        }
    }

    /// A second handle over the *same* backing bytes with a fresh, disarmed
    /// policy — simulates reopening the file after a crash.
    #[must_use]
    pub fn reopen(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
            policy: Arc::new(FaultPolicy::new()),
        }
    }

    /// A clone of the shared fault policy, to arm/disarm from the test.
    pub fn policy(&self) -> Arc<FaultPolicy> {
        Arc::clone(&self.policy)
    }

    /// A snapshot copy of the current bytes (e.g. to build a pristine image the
    /// fuzz clones and corrupts).
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        self.data.lock().unwrap().clone()
    }

    /// Mutate the backing bytes in place (e.g. to corrupt a specific page after a
    /// commit, then reopen over the damage). The closure's return value is passed
    /// through.
    pub fn with_bytes_mut<R>(&self, f: impl FnOnce(&mut Vec<u8>) -> R) -> R {
        let mut data = self.data.lock().unwrap();
        f(&mut data)
    }

    /// Faithful positioned write into the buffer (extends + zero-fills as needed).
    fn write_through(data: &mut Vec<u8>, offset: u64, buf: &[u8]) {
        let end = offset as usize + buf.len();
        if data.len() < end {
            data.resize(end, 0);
        }
        data[offset as usize..end].copy_from_slice(buf);
    }
}

impl Default for MemBlockIo {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockIo for MemBlockIo {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        if offset >= DATA_REGION_START
            && self
                .policy
                .failing_data_reads
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
        {
            return Err(std::io::Error::other("injected data-page read failure"));
        }
        if offset >= DATA_REGION_START
            && let Some(hook) = self.policy.before_data_read.take_for_read()
        {
            hook(offset);
        }
        let data = self.data.lock().unwrap();
        let start = offset as usize;
        if start >= data.len() {
            return Ok(0);
        }
        let end = (start + buf.len()).min(data.len());
        let n = end - start;
        buf[..n].copy_from_slice(&data[start..end]);
        Ok(n)
    }

    fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
        // A data-page write (at or above the data region) is a fault candidate only
        // under `FailDataWritesAfter`; the slot modes target the meta slot, and the sync
        // modes the syncs.
        if offset >= DATA_REGION_START {
            if let FaultMode::FailDataWritesAfter { successes } = self.policy.current()
                && self.policy.data_writes_seen.fetch_add(1, Ordering::SeqCst) >= successes
            {
                if self.policy.data_write_once.load(Ordering::SeqCst) == 1 {
                    self.policy.mode.store(0, Ordering::SeqCst);
                }
                return Err(self.policy.injected("data-page write failure"));
            }
            let mut data = self.data.lock().unwrap();
            Self::write_through(&mut data, offset, buf);
            return Ok(());
        }
        match self.policy.current() {
            FaultMode::DropSlotWrites => {
                // The meta write never reaches disk. Report success (a crash
                // gives the caller no error) but write nothing.
                return Ok(());
            }
            FaultMode::TearNextSlotWrite { prefix } => {
                // Fire once: tear this write to `prefix` bytes, then revert.
                if self.policy.tear_fired.swap(1, Ordering::SeqCst) == 0 {
                    let cut = prefix.min(buf.len());
                    let mut data = self.data.lock().unwrap();
                    Self::write_through(&mut data, offset, &buf[..cut]);
                    self.policy.mode.store(0, Ordering::SeqCst);
                    return Ok(());
                }
            }
            FaultMode::FailSlotWrites => {
                // The meta write fails loud — the caller gets an error and must
                // surface it rather than reporting a durable commit.
                return Err(std::io::Error::other("injected meta-slot write failure"));
            }
            // None of the data-write or sync modes changes a meta write, and `None` is
            // faithful by definition.
            FaultMode::FailDataWritesAfter { .. }
            | FaultMode::FailSyncs
            | FaultMode::FailSyncsAfter { .. }
            | FaultMode::None => {}
        }
        let mut data = self.data.lock().unwrap();
        Self::write_through(&mut data, offset, buf);
        Ok(())
    }

    fn sync(&self) -> std::io::Result<()> {
        match self.policy.current() {
            FaultMode::FailSyncs => return Err(std::io::Error::other("injected sync failure")),
            FaultMode::FailSyncsAfter { successes }
                if self.policy.syncs_seen.fetch_add(1, Ordering::SeqCst) >= successes =>
            {
                return Err(std::io::Error::other("injected sync failure (writeback)"));
            }
            _ => {}
        }
        // Under DropSlotWrites the meta fsync is a no-op too: the commit point
        // never becomes durable. (The earlier data fsync is irrelevant here — the
        // bytes it would flush are already in the shared buffer.)
        Ok(())
    }

    fn len(&self) -> std::io::Result<u64> {
        Ok(self.data.lock().unwrap().len() as u64)
    }

    fn set_len(&self, size: u64) -> std::io::Result<()> {
        self.data.lock().unwrap().resize(size as usize, 0);
        Ok(())
    }
}
