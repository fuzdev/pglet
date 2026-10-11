//! Copy-on-Write B+tree implementation.
//!
//! Adds `CoW` semantics to the core B+tree for MVCC support.
//!
//! # Key Concepts
//!
//! - **Never modify in place**: All writes create new page copies
//! - **Root versioning**: Each commit creates a new root; old roots are snapshots
//! - **Atomic commits**: Root pointer swap is the commit point
//! - **Garbage collection**: Old pages freed when no readers reference them
//!
//! # Write Path
//!
//! ```text
//! 1. Acquire write lock (single-writer)
//! 2. Find path from root to target leaf
//! 3. Clone leaf, apply modification
//! 4. Clone parents up to root, updating child pointers
//! 5. Atomic root swap (commit point)
//! 6. Register old pages for GC
//! 7. Release write lock
//! ```
//!
//! # Read Path
//!
//! ```text
//! 1. Capture current root (atomic read)
//! 2. Traverse tree using captured root
//! 3. No locks needed - pages are immutable once written
//! ```

use super::BtreeKey;
use super::page::{
    DATA_SIZE, INLINE_VALUE_MAX, InternalEntry, LeafEntry, LeafValue, MAX_KEY_LEN, NULL_PAGE_ID,
    OverflowRef, Page, PageFlags, PageHeader, PageId, encode_value, encoded_value_len,
    internal_entries_fit, internal_entry_bytes, leaf_entries_fit, leaf_entry_bytes,
    leaf_entry_bytes_inline, leaf_entry_bytes_out_of_line, split_points,
};
use super::store::{PageStore, PageStoreError, PageStoreResult};
use pglet_sql::types::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use parking_lot::{Mutex, RwLock};

/// Version number for root snapshots.
pub type Version = u64;

/// `CoW` B+tree errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CowBtreeError {
    /// Key not found.
    NotFound(BtreeKey),
    /// Page not found in storage.
    PageNotFound(PageId),
    /// Tree is empty.
    EmptyTree,
    /// A single leaf entry is larger than one page can hold even with its value out
    /// of line — a key too long to sit beside an overflow reference; the insert is
    /// rejected rather than overflowing the page buffer.
    ValueTooLarge,
    /// A key longer than [`MAX_KEY_LEN`], which could not be promoted into an
    /// internal page as a separator; the insert is rejected before the tree is
    /// touched. `len` is the key's encoded length.
    KeyTooLarge {
        /// The key's encoded length in bytes.
        len: usize,
    },
    /// A page-store read failed (e.g. checksum corruption); carries the typed
    /// underlying error so corruption surfaces loud instead of as a generic
    /// page-not-found.
    Storage(PageStoreError),
    /// Internal error.
    Internal(String),
}

impl std::fmt::Display for CowBtreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(key) => write!(f, "key not found: {key:?}"),
            Self::PageNotFound(id) => write!(f, "page not found: {id}"),
            Self::EmptyTree => write!(f, "tree is empty"),
            Self::ValueTooLarge => write!(
                f,
                "entry too large: a key with its value reference exceeds one page"
            ),
            Self::KeyTooLarge { len } => write!(
                f,
                "key too large: {len} encoded bytes exceeds the {MAX_KEY_LEN} a B+tree page can hold"
            ),
            Self::Storage(e) => write!(f, "page store error: {e}"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for CowBtreeError {}

impl From<PageStoreError> for CowBtreeError {
    fn from(e: PageStoreError) -> Self {
        Self::Storage(e)
    }
}

pub type Result<T> = std::result::Result<T, CowBtreeError>;

/// The right siblings a node split produced, left to right: each is `(separator
/// key, page)`. Empty when the node did not split; more than one when no two-way
/// cut fits (see `page::split_points`).
type CowSplit = Vec<(BtreeKey, PageId)>;

/// Batch of changes to apply atomically.
///
/// Used by `CowBtree::apply_batch()` for transactional commits.
#[derive(Debug, Default)]
pub struct BatchChanges {
    /// Key-value pairs to insert.
    pub inserts: Vec<(BtreeKey, Vec<Value>)>,
    /// Keys to delete.
    pub deletes: Vec<BtreeKey>,
}

impl BatchChanges {
    /// Create an empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the batch is empty.
    pub const fn is_empty(&self) -> bool {
        self.inserts.is_empty() && self.deletes.is_empty()
    }

    /// Add an insert to the batch.
    pub fn insert(&mut self, key: BtreeKey, value: Vec<Value>) {
        self.inserts.push((key, value));
    }

    /// Add a delete to the batch.
    pub fn delete(&mut self, key: BtreeKey) {
        self.deletes.push(key);
    }

    /// Get the total number of operations.
    pub const fn len(&self) -> usize {
        self.inserts.len() + self.deletes.len()
    }
}

/// Page store with `CoW` semantics.
///
/// Pages are immutable once written. Modifications create new pages.
#[derive(Debug)]
pub struct CowPageStore {
    /// All pages (immutable once inserted).
    pages: RwLock<HashMap<PageId, Page>>,
    /// Next page ID to allocate.
    next_page_id: AtomicU64,
    /// Free page IDs available for reuse.
    free_list: Mutex<Vec<PageId>>,
}

impl CowPageStore {
    pub fn new() -> Self {
        Self {
            pages: RwLock::new(HashMap::new()),
            next_page_id: AtomicU64::new(0),
            free_list: Mutex::new(Vec::new()),
        }
    }
}

impl PageStore for CowPageStore {
    fn allocate(&self) -> PageId {
        // Try free list first; bind the pop so the guard drops before the branch.
        let reused = self.free_list.lock().pop();
        if let Some(id) = reused {
            return id;
        }
        self.next_page_id.fetch_add(1, Ordering::SeqCst)
    }

    fn get(&self, page_id: PageId) -> PageStoreResult<Option<Page>> {
        Ok(self.pages.read().get(&page_id).cloned())
    }

    /// Store a page, refusing one that would not fit a page on disk.
    ///
    /// This store never serializes, so without the check it would hold a page the
    /// file store cannot write and answer from it — the in-memory tree (the fixture
    /// harness's, the wasm `PgletBtree`'s) must refuse what the file store refuses,
    /// or neither guards the other.
    fn put(&self, page: Page) -> PageStoreResult<()> {
        page.check_fits()?;
        let id = page.page_id();
        self.pages.write().insert(id, page);
        Ok(())
    }

    /// Check every page once, then store them all under one lock — `put`'s own
    /// check is the one the default would repeat per page.
    fn put_batch(&self, pages: Vec<Page>) -> PageStoreResult<()> {
        for page in &pages {
            page.check_fits()?;
        }
        let mut map = self.pages.write();
        for page in pages {
            map.insert(page.page_id(), page);
        }
        Ok(())
    }

    fn free_pages(&self, page_ids: Vec<PageId>) {
        self.free_list.lock().extend(page_ids);
    }

    fn page_count(&self) -> usize {
        self.pages.read().len()
    }

    fn free_list_size(&self) -> usize {
        self.free_list.lock().len()
    }

    fn next_page_id(&self) -> PageId {
        self.next_page_id.load(Ordering::SeqCst)
    }

    fn free_list(&self) -> Vec<PageId> {
        self.free_list.lock().clone()
    }

    fn fork_in_memory(&self) -> Option<Box<dyn PageStore>> {
        // Clone the whole store: the page map (each `Page` clone bumps its entry
        // vector's `Arc` refcount, not a deep value copy), the allocation counter,
        // and the free list. The clone is an independent store with the same
        // committed content; a write on either store CoWs the touched page
        // (`Arc::make_mut`), so they never observe each other's mutations.
        Some(Box::new(Self {
            pages: RwLock::new(self.pages.read().clone()),
            next_page_id: AtomicU64::new(self.next_page_id.load(Ordering::SeqCst)),
            free_list: Mutex::new(self.free_list.lock().clone()),
        }))
    }
}

impl Default for CowPageStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Which child pointer of an internal node a traversal descended through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildSlot {
    /// The leftmost child pointer (`first_child`).
    First,
    /// The child pointer of `entries[index]`.
    Entry(usize),
}

/// Path element tracking parent-child relationship during traversal.
#[derive(Debug, Clone)]
struct PathElement {
    page_id: PageId,
    /// Which child pointer we descended through.
    child_slot: ChildSlot,
}

/// Copy-on-first-write working set for a single `apply_batch` commit.
///
/// A naive CoW batch re-copies every page on the root-to-leaf path for *each*
/// insert/delete, so the shared upper/internal pages are re-versioned once per
/// op — write amplification of roughly tree-depth per row, with every
/// intermediate version becoming immediate garbage.
///
/// This working set fixes that by copying each pre-batch page **at most once**
/// per batch. The first time an op touches a page that existed before the batch
/// (`prebatch` here), it is copied to a fresh page id and recorded in `pages`
/// and `created`. Any later op in the same batch that needs to modify that page
/// finds it in `created` and mutates the in-memory copy in place — no new
/// allocation, no extra version. Pages allocated for splits within the batch are
/// likewise `created`, so subsequent ops mutate them directly too.
///
/// MVCC safety: `created` is keyed on the *new* page identity, so in-place
/// mutation only ever touches pages allocated fresh in this batch. Pages
/// reachable from any committed/old root are never mutated — readers holding an
/// old root traverse an entirely immutable tree. At commit, every page in
/// `pages` is written exactly once via `store.put`, and each superseded pre-batch
/// page is freed exactly once.
struct BatchWorkingSet {
    /// New/dirtied pages built during this batch, keyed by their new page id.
    /// These are the pages that will be written to the store at commit.
    pages: HashMap<PageId, Page>,
    /// Page ids allocated (or first-copied) within this batch. Modifying one of
    /// these is done in place — it has no committed predecessor to protect.
    created: HashSet<PageId>,
    /// Pre-batch page ids that have been superseded by a new version and must be
    /// freed once at commit. A pre-batch page is recorded here the first time it
    /// is copied; subsequent ops reuse the existing copy and add nothing.
    superseded: Vec<PageId>,
    /// The pre-batch overflow-chain links this batch has released (superseded), so
    /// a second release of one — two entries naming the same chain, which only a
    /// damaged store holds — is refused rather than superseding the page twice.
    released: HashSet<PageId>,
}

impl BatchWorkingSet {
    fn new() -> Self {
        Self {
            pages: HashMap::new(),
            created: HashSet::new(),
            superseded: Vec::new(),
            released: HashSet::new(),
        }
    }

    /// Read the current state of a page within this batch: the working copy if
    /// one exists, otherwise the committed page from the store. Use this for
    /// **read-only** descent — a working-copy hit is *cloned* (the page stays in
    /// `pages`), which is fine for reading but would leave two `Arc` references and
    /// so defeat the in-place `make_mut` of [`Self::take_for_write`].
    fn get(&self, store: &dyn PageStore, page_id: PageId) -> Result<Page> {
        if let Some(page) = self.pages.get(&page_id) {
            return Ok(page.clone());
        }
        store
            .get(page_id)?
            .ok_or(CowBtreeError::PageNotFound(page_id))
    }

    /// Obtain an owned, **uniquely-referenced** copy of a page for mutation, so a
    /// subsequent `Arc::make_mut` on its entries mutates **in place** rather than
    /// deep-copying the whole page on every touch.
    ///
    /// - A page **already created in this batch** is *removed* from `pages` and
    ///   returned by value — ws is now its sole owner (its entry `Arc` refcount is
    ///   1), so `make_mut` won't copy. The caller re-inserts it via
    ///   [`write_dirtied`](CowBtree::write_dirtied) (created → same id, no new
    ///   supersede). This is the fix for the batched-write cliff: without it,
    ///   [`get`](Self::get) would clone the page (leaving it in `pages`), inflating
    ///   the refcount to 2 so every insert re-cloned every page on its path.
    /// - A **pre-batch** page is read from the store; its entries `Arc` is still
    ///   shared with the page cache, so the caller's `make_mut` deep-copies it —
    ///   the one necessary copy-on-first-write that keeps old roots immutable. The
    ///   caller's `write_dirtied` allocates a new id and supersedes the old one.
    ///
    /// The `created` set is intentionally left untouched by the remove, so
    /// `write_dirtied` still sees the page as created and reuses its id in place.
    fn take_for_write(&mut self, store: &dyn PageStore, page_id: PageId) -> Result<Page> {
        if let Some(page) = self.pages.remove(&page_id) {
            return Ok(page);
        }
        store
            .get(page_id)?
            .ok_or(CowBtreeError::PageNotFound(page_id))
    }

    /// Was `page_id` created (or first-copied) within this batch? If so it may be
    /// mutated in place.
    fn is_created(&self, page_id: PageId) -> bool {
        self.created.contains(&page_id)
    }

    /// Record a freshly created (in-batch) page. Mutating it later is in place.
    fn insert_created(&mut self, page: Page) {
        self.created.insert(page.page_id());
        self.pages.insert(page.page_id(), page);
    }

    /// Mark a pre-batch page as superseded (to be freed once at commit).
    fn supersede(&mut self, page_id: PageId) {
        self.superseded.push(page_id);
    }

    /// Drop a page this batch created and no longer references: it is never
    /// written, and its id goes straight back to the store's free list, since no
    /// root has ever reached it.
    fn discard_created(&mut self, store: &dyn PageStore, page_id: PageId) {
        self.pages.remove(&page_id);
        self.created.remove(&page_id);
        store.free_pages(vec![page_id]);
    }
}

/// The pages one tree walk has entered, so a page reached twice — a page naming an
/// ancestor, or two pages naming one child, which no well-formed tree holds — is refused
/// where the walk meets it rather than followed. That bounds every walk by the pages
/// the store holds: a descent cannot loop, and a scan cannot emit a subtree twice.
///
/// A descent enters a handful of pages, so the first few are kept in a fixed array
/// and a set is built only past it, keeping a point read free of an allocation.
struct Visited {
    inline: [PageId; Self::INLINE],
    len: usize,
    spill: HashSet<PageId>,
}

impl Visited {
    const INLINE: usize = 16;

    fn new() -> Self {
        Self {
            inline: [NULL_PAGE_ID; Self::INLINE],
            len: 0,
            spill: HashSet::new(),
        }
    }

    /// Record that the walk enters `page`, or refuse a page it has entered before.
    fn enter(&mut self, page: PageId) -> Result<()> {
        let held = &self.inline[..self.len.min(Self::INLINE)];
        if held.contains(&page) || self.spill.contains(&page) {
            return Err(CowBtreeError::Storage(PageStoreError::CorruptTree { page }));
        }
        if self.len < Self::INLINE {
            self.inline[self.len] = page;
        } else {
            self.spill.insert(page);
        }
        self.len += 1;
        Ok(())
    }
}

/// Where an insert's value goes, decided before the batch touches the tree.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Placement {
    /// On the leaf, as its postcard encoding.
    Inline,
    /// In an overflow chain, the leaf holding a reference.
    OutOfLine,
}

/// Decide where `value` is stored beside `key`, or refuse an entry no page can hold.
///
/// A value whose encoding is over [`INLINE_VALUE_MAX`] goes out of line, and so does
/// one short enough whose entry would still not fit a page inline (a long key). A key
/// over [`MAX_KEY_LEN`] could not be promoted into an internal page as a separator,
/// and an entry that does not fit a page even with an out-of-line reference could
/// not be placed at all; both are refused. With every entry fitting a page alone,
/// every page the split builds fits (`page::split_points`).
fn placement(key: &BtreeKey, value: &[Value]) -> Result<Placement> {
    let key_len = key.encoded_len();
    if key_len > MAX_KEY_LEN {
        return Err(CowBtreeError::KeyTooLarge { len: key_len });
    }
    let value_len = encoded_value_len(value);
    if value_len <= INLINE_VALUE_MAX && leaf_entry_bytes_inline(key_len, value_len) <= DATA_SIZE {
        return Ok(Placement::Inline);
    }
    if leaf_entry_bytes_out_of_line(key_len) > DATA_SIZE {
        return Err(CowBtreeError::ValueTooLarge);
    }
    Ok(Placement::OutOfLine)
}

/// Default vacuum threshold (pending pages count).
const DEFAULT_VACUUM_THRESHOLD: usize = 100;

/// Copy-on-Write B+tree with MVCC support.
///
/// Supports concurrent readers with snapshot isolation.
/// Single writer at a time (enforced by `write_lock`).
///
/// The page store backend is abstracted via trait object, supporting both
/// in-memory (default) and persistent (OPFS) storage.
// Note: Manual Debug impl below (Mutex/RwLock don't derive Debug nicely)
pub struct CowBtree {
    /// Page storage backend (trait object for flexibility).
    store: Box<dyn PageStore>,
    /// Current root page ID (atomic for lock-free reads).
    root: AtomicU64,
    /// Current version number.
    version: AtomicU64,
    /// Seqlock generation counter guarding the (root, version) pair: even =
    /// stable, odd = a publish is in progress. A lock-free `snapshot()` reader
    /// spins while it is odd (or if it changes across the read), so it never
    /// observes a torn (old root + new version, or vice versa) pair.
    generation: AtomicU64,
    /// Single-writer lock.
    write_lock: Mutex<()>,
    /// Active snapshots: version -> reference count. Pages from versions with
    /// active snapshots cannot be freed.
    ///
    /// `parking_lot::RwLock` (the read-path hot lock) with **atomic** counts: a
    /// `snapshot`/`release` for an already-tracked version takes only the *shared*
    /// read lock and a `fetch_add`/`fetch_sub` on its counter, so concurrent
    /// readers don't serialize (the old `RwLock<HashMap<_, usize>>` took the
    /// *write* lock on every snapshot and release). The exclusive write lock is
    /// taken only to insert a never-seen version or remove a drained one. The
    /// `min_active_version` contract is unchanged: the minimum version whose count
    /// is `> 0` is `<=` every active snapshot's version, so vacuum still never
    /// frees a page a reader can reach.
    active_snapshots: RwLock<HashMap<Version, AtomicUsize>>,
    /// Pages pending garbage collection: (`page_ids`, `version_freed_at`), in the order
    /// `apply_batch` published them — so the versions rise, which a save relies on
    /// (`PageStore::save_state`); collection removes batches without reordering them.
    pending_gc: Mutex<Vec<(Vec<PageId>, Version)>>,
    /// Lock-free shadow of the total page count across `pending_gc` batches, kept
    /// in step with the `Vec` under the same `pending_gc` lock. `release_snapshot`
    /// reads it to decide whether to *attempt* GC without locking `pending_gc` on
    /// every release; `try_gc` re-derives the truth under the lock, so a stale read
    /// only shifts GC timing, never correctness.
    pending_gc_pages: AtomicUsize,
    /// Vacuum threshold: trigger auto-vacuum when pending pages exceed this.
    vacuum_threshold: usize,
}

impl CowBtree {
    /// Create a new empty `CoW` B+tree with in-memory storage.
    pub fn new() -> Self {
        Self::with_vacuum_threshold(DEFAULT_VACUUM_THRESHOLD)
    }

    /// Create a new empty `CoW` B+tree with a custom vacuum threshold.
    ///
    /// Uses in-memory storage. The vacuum threshold controls when automatic
    /// garbage collection runs. When pending pages exceed this threshold,
    /// vacuum is triggered on snapshot release.
    pub fn with_vacuum_threshold(vacuum_threshold: usize) -> Self {
        Self {
            store: Box::new(CowPageStore::new()),
            root: AtomicU64::new(NULL_PAGE_ID),
            version: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            write_lock: Mutex::new(()),
            active_snapshots: RwLock::new(HashMap::new()),
            pending_gc: Mutex::new(Vec::new()),
            pending_gc_pages: AtomicUsize::new(0),
            vacuum_threshold,
        }
    }

    /// Create a new B+tree with a custom page store.
    ///
    /// Use this constructor when you need a specific storage backend
    /// (e.g., OPFS for browser persistence).
    ///
    /// # Arguments
    /// * `store` - The page store backend
    /// * `root` - Initial root page ID (use `NULL_PAGE_ID` for empty tree)
    /// * `version` - Initial version number
    /// * `vacuum_threshold` - Pages pending before auto-vacuum triggers
    pub fn with_store(
        store: Box<dyn PageStore>,
        root: PageId,
        version: u64,
        vacuum_threshold: usize,
    ) -> Self {
        Self {
            store,
            root: AtomicU64::new(root),
            version: AtomicU64::new(version),
            generation: AtomicU64::new(0),
            write_lock: Mutex::new(()),
            active_snapshots: RwLock::new(HashMap::new()),
            pending_gc: Mutex::new(Vec::new()),
            pending_gc_pages: AtomicUsize::new(0),
            vacuum_threshold,
        }
    }

    /// Get the current vacuum threshold.
    pub const fn vacuum_threshold(&self) -> usize {
        self.vacuum_threshold
    }

    /// Get current version.
    pub fn current_version(&self) -> Version {
        self.version.load(Ordering::Acquire)
    }

    /// Get current root page ID.
    pub fn current_root(&self) -> PageId {
        self.root.load(Ordering::Acquire)
    }

    /// Publish a new (root, version) pair as one seqlock-protected unit, so a
    /// lock-free `snapshot()` reader never observes a torn pair (old root + new
    /// version, or vice versa). The caller holds `write_lock` (single writer), so
    /// the generation bumps need no CAS — the odd/even fence only orders the
    /// reader. This is the *only* place the (root, version) pair is mutated, so no
    /// publish site can drift out of step.
    fn publish_root(&self, new_root: PageId, new_version: Version) {
        self.generation.fetch_add(1, Ordering::Release); // enter: now ODD
        self.root.store(new_root, Ordering::Release);
        self.version.store(new_version, Ordering::Release);
        self.generation.fetch_add(1, Ordering::Release); // leave: now EVEN
    }

    /// Check if tree is empty.
    pub fn is_empty(&self) -> bool {
        self.current_root() == NULL_PAGE_ID
    }

    /// Get a reference to the underlying page store.
    pub fn store(&self) -> &dyn PageStore {
        &*self.store
    }

    /// Flush the underlying page store.
    ///
    /// For in-memory stores, this is a no-op.
    /// For persistent stores (OPFS), this ensures data is written to disk. A refused
    /// flush fails the store ([`Self::settle_refusal`]): nothing here says what the
    /// flushed writes belonged to.
    pub fn flush(&self) -> Result<()> {
        self.store
            .flush()
            .map_err(|e| self.settle_refusal(CowBtreeError::Storage(e), false))
    }

    /// Decide what a write or `fsync` the device refused ([`PageStoreError::WriteRefused`])
    /// does to the store: leave it serving when the refused writes can be dropped with
    /// nothing lost, or fail it ([`PageStoreError::WriteFailed`], recorded in the store's
    /// [`WriteFailure`](super::store::WriteFailure)).
    ///
    /// Droppable means the caller said so (`discardable`): the batch is the whole of its
    /// statement's or block's effect that nothing else takes back — anything else it did,
    /// published or not, a savepoint rollback has already undone or its host's rollback
    /// restores. The batch that met the refusal published nothing, so dropping it leaves
    /// the tree as it was. Whatever else the tree has published since the last save —
    /// another connection's open block's DDL, say — stays published and is made durable by
    /// the next save, as it would have been had nothing been refused: whether the store
    /// fails is decided by the refused statement's own effects. Otherwise the refused
    /// batch may belong with a published one — a statement that published a batch before
    /// this one — and dropping it would leave that statement half applied where readers
    /// see it and the next save persists it, while taking the published version back is
    /// unsound: a lock-free reader may already hold its root; so the store fails. Any
    /// other error passes through.
    fn settle_refusal(&self, error: CowBtreeError, discardable: bool) -> CowBtreeError {
        let CowBtreeError::Storage(PageStoreError::WriteRefused(refused)) = error else {
            return error;
        };
        if discardable {
            return CowBtreeError::Storage(PageStoreError::WriteRefused(refused));
        }
        if let Some(failure) = self.store.write_failure() {
            failure.record(refused.reason.clone());
        }
        CowBtreeError::Storage(PageStoreError::WriteFailed(refused))
    }

    /// Fork this tree into an independent in-memory tree at the current committed
    /// `(root, version)`. Returns `None` if the backing store isn't in-memory
    /// forkable (e.g. a file/OPFS-backed `DoubleMetaStore`).
    ///
    /// The forked tree shares the source's pages by `Arc` refcount (no row deep
    /// copy) but has its own root, version, GC state, and page-id space, so a
    /// write on either tree copies only the pages it touches and is invisible to
    /// the other — the per-test-fixture fork primitive. The source is captured
    /// through a pinned snapshot so a concurrent commit + vacuum can't free a
    /// reachable page mid-clone; any in-flight transaction's pending writes (which
    /// live outside the tree, in the wrapper) are not carried — a fork reflects
    /// committed state.
    pub fn fork(&self) -> Option<Self> {
        // Pin (root, version) as a consistent seqlock pair and protect its pages
        // from GC while we clone the store.
        let snap = self.snapshot();
        let Some(store) = self.store.fork_in_memory() else {
            self.release_snapshot(&snap);
            return None;
        };
        let forked = Self::with_store(store, snap.root, snap.version, self.vacuum_threshold);
        self.release_snapshot(&snap);
        Some(forked)
    }

    // ========================================================================
    // Snapshot Management (for MVCC)
    // ========================================================================

    /// Create a snapshot at current version.
    /// Returns the version number to use for reads.
    pub fn snapshot(&self) -> Snapshot {
        loop {
            // Seqlock read: spin until we observe a (version, root) pair published
            // under a single even generation, so the pair is never torn (old root
            // + new version, or vice versa).
            let (version, root) = loop {
                let g1 = self.generation.load(Ordering::Acquire);
                if g1 & 1 != 0 {
                    std::hint::spin_loop(); // a publish is in progress; retry
                    continue;
                }
                let version = self.version.load(Ordering::Acquire);
                let root = self.root.load(Ordering::Acquire);
                if self.generation.load(Ordering::Acquire) == g1 {
                    break (version, root); // generation unchanged → consistent pair
                }
                std::hint::spin_loop();
            };

            // Pin this version's pages against vacuum, THEN re-validate that the
            // version is still current. `apply_batch` publishes the next version
            // *before* pushing the retiring pages to `pending_gc`, so if the
            // version has not advanced since our capture, this version's pages
            // cannot have been freed yet — and our now-registered refcount keeps
            // any later vacuum from freeing them (`min_active_version <= version`).
            // Without this re-check there is a window between the seqlock capture
            // and the refcount bump in which a commit + vacuum could free the
            // captured root's pages out from under us (they would be reused and the
            // scan would read another version's data).
            self.increment_snapshot_ref(version);
            if self.version.load(Ordering::Acquire) == version {
                return Snapshot { version, root };
            }
            // The version advanced during capture: our refcount may be too late to
            // have protected the captured root. Release it and retry with the
            // newer state.
            self.decrement_snapshot_ref(version);
        }
    }

    /// Increment the active-snapshot reference count for `version`. Fast path: the
    /// version is already tracked, so a *shared* read lock + an atomic bump
    /// suffices — concurrent readers don't serialize. Slow path (a never-seen
    /// version): take the write lock to insert the counter (`entry` tolerates a
    /// racing insert from another thread).
    fn increment_snapshot_ref(&self, version: Version) {
        let tracked = self.active_snapshots.read();
        if let Some(count) = tracked.get(&version) {
            count.fetch_add(1, Ordering::SeqCst);
        } else {
            drop(tracked);
            self.active_snapshots
                .write()
                .entry(version)
                .or_insert_with(|| AtomicUsize::new(0))
                .fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Release a snapshot.
    ///
    /// Automatically triggers garbage collection if pending pages exceed
    /// the vacuum threshold.
    pub fn release_snapshot(&self, snapshot: &Snapshot) {
        self.decrement_snapshot_ref(snapshot.version);

        // Gate GC on the lock-free page-count shadow, so the common (under
        // threshold) release never locks `pending_gc`; the collection re-checks under
        // the lock. A reader does not wait for it: while a save holds the pending set
        // (`save_state`), this collection is skipped and a later one frees the pages.
        if self.pending_gc_pages.load(Ordering::Relaxed) >= self.vacuum_threshold
            && let Some(pending) = self.pending_gc.try_lock()
        {
            self.collect(pending);
        }
    }

    /// Release a snapshot and force garbage collection.
    ///
    /// Unlike `release_snapshot()`, this always runs GC regardless of threshold.
    pub fn release_snapshot_and_vacuum(&self, snapshot: &Snapshot) -> VacuumResult {
        self.decrement_snapshot_ref(snapshot.version);
        self.try_gc()
    }

    /// Decrement the active-snapshot reference count for `version` (also used by
    /// `snapshot`'s retry path to undo a registration whose version went stale).
    fn decrement_snapshot_ref(&self, version: Version) {
        // Fast path: a *shared* read lock + an atomic decrement.
        let dropped_to_zero = {
            let tracked = self.active_snapshots.read();
            // `fetch_sub` returns the *previous* value; `1` means this was the last
            // reference, so the version is now a removal candidate.
            tracked
                .get(&version)
                .is_some_and(|count| count.fetch_sub(1, Ordering::SeqCst) == 1)
        };
        if !dropped_to_zero {
            return;
        }
        // Last reference dropped: take the write lock and remove the entry — but
        // only if it is *still* zero. A concurrent `snapshot` of the same version
        // may have re-incremented it after our `fetch_sub` (and before we acquired
        // the write lock); removing it then would lose that reader's reference and
        // could let `min_active_version` rise above a live snapshot. Re-checking
        // under the exclusive lock closes that race.
        let mut tracked = self.active_snapshots.write();
        if let Some(count) = tracked.get(&version)
            && count.load(Ordering::SeqCst) == 0
        {
            tracked.remove(&version);
        }
    }

    /// Get minimum active snapshot version — the smallest version still holding a
    /// reader. A transient zero-count entry (a release that dropped to zero but
    /// has not yet been removed) is skipped, so the result is `<=` every active
    /// snapshot's version, which is what vacuum's safety relies on.
    fn min_active_version(&self) -> Option<Version> {
        self.active_snapshots
            .read()
            .iter()
            .filter(|(_, count)| count.load(Ordering::SeqCst) > 0)
            .map(|(version, _)| *version)
            .min()
    }

    /// Try to garbage collect old pages.
    /// Returns statistics about the vacuum operation.
    ///
    /// A page is freed once no version a reader or a reopen can reach holds it: its
    /// `freed_version` — the last version holding it — must be below the oldest
    /// active snapshot's version (the current version when none is active) **and**
    /// below the store's durable version. The second bound is what a statement that
    /// publishes several versions before its commit's meta slot is written needs: the
    /// versions it published in memory are not the one a crash reopens, and a page the
    /// durable version reaches is overwritten by the next allocation once freed.
    fn try_gc(&self) -> VacuumResult {
        let pending = self.pending_gc.lock();
        self.collect(pending)
    }

    /// [`Self::try_gc`] over a pending set already locked. The pages it frees reach the
    /// store's free list **before** the lock is released, so a save — which reads the
    /// pending set under the same lock — sees each superseded page either pending or
    /// free, never neither.
    fn collect(
        &self,
        mut pending: parking_lot::MutexGuard<'_, Vec<(Vec<PageId>, Version)>>,
    ) -> VacuumResult {
        let min_version = self.min_active_version();
        // No active snapshots: can GC everything up to the current version.
        let mut gc_threshold = min_version.unwrap_or_else(|| self.current_version());
        if let Some(durable) = self.store.durable_version() {
            gc_threshold = gc_threshold.min(durable);
        }

        let (pages_to_free, pages_pending) = {
            let mut to_free = Vec::new();
            let mut still_pending = 0;
            pending.retain(|(pages, freed_version)| {
                if *freed_version < gc_threshold {
                    to_free.extend(pages.iter().copied());
                    false // Remove from pending
                } else {
                    still_pending += pages.len();
                    true // Keep in pending
                }
            });
            // Keep the lock-free page-count shadow in step with the Vec, under the
            // same lock that mutates it.
            self.pending_gc_pages
                .fetch_sub(to_free.len(), Ordering::Relaxed);
            (to_free, still_pending)
        };

        let pages_freed = pages_to_free.len();
        if !pages_to_free.is_empty() {
            self.store.free_pages(pages_to_free);
        }
        drop(pending);

        VacuumResult {
            pages_freed,
            pages_pending,
            min_active_version: min_version,
        }
    }

    /// Make the current version durable: the store's meta-slot flip
    /// ([`PageStore::save_state`]) over the current root and version, handing it the
    /// pages superseded and not yet collected.
    ///
    /// The tree is the one owner of that set (`pending_gc`); the save reads it under
    /// the writer lock and the pending lock, so no batch publishes and no collection
    /// frees between what the save logs and what it persists as free.
    ///
    /// # Errors
    ///
    /// The store's error, as [`CowBtreeError::Storage`].
    pub fn save_state(&self, next_table_id: u32) -> Result<()> {
        let _writer = self.write_lock.lock();
        let pending = self.pending_gc.lock();
        self.store
            .save_state(
                self.current_root(),
                self.current_version(),
                next_table_id,
                &pending,
            )
            .map_err(CowBtreeError::Storage)
    }

    /// Run garbage collection explicitly.
    ///
    /// This is the public API for vacuum. It frees pages that are no longer
    /// referenced by any active snapshot.
    ///
    /// Returns statistics about the vacuum operation.
    pub fn vacuum(&self) -> VacuumResult {
        self.try_gc()
    }

    /// Get the number of pages pending garbage collection.
    pub fn pending_gc_count(&self) -> (usize, usize) {
        let pending = self.pending_gc.lock();
        let batches = pending.len();
        let pages: usize = pending.iter().map(|(pages, _)| pages.len()).sum();
        (batches, pages)
    }

    // ========================================================================
    // Read Operations (lock-free)
    // ========================================================================

    /// Get a value by key using current root.
    pub fn get(&self, key: &BtreeKey) -> Result<Option<Vec<Value>>> {
        let root = self.current_root();
        self.get_at_root(root, key)
    }

    /// Get a value by key using a specific root (for snapshots).
    pub fn get_at_root(&self, root: PageId, key: &BtreeKey) -> Result<Option<Vec<Value>>> {
        if root == NULL_PAGE_ID {
            return Ok(None);
        }

        let leaf_id = self.find_leaf_from(root, key)?;
        let leaf = self
            .store
            .get(leaf_id)?
            .ok_or(CowBtreeError::PageNotFound(leaf_id))?;

        if let Page::Leaf { entries, .. } = leaf {
            entries
                .binary_search_by(|e| e.key.cmp(key))
                .map_or(Ok(None), |idx| {
                    self.read_value(&entries[idx].value).map(Some)
                })
        } else {
            Err(CowBtreeError::PageNotFound(leaf_id))
        }
    }

    /// A leaf value as its entry stores it: an inline value cloned, an out-of-line
    /// one read back from its overflow chain.
    fn read_value(&self, value: &LeafValue) -> Result<Vec<Value>> {
        match value {
            LeafValue::Inline(v) => Ok(v.clone()),
            LeafValue::Overflow(r) => self.read_overflow(*r),
        }
    }

    /// Read an out-of-line value back from its overflow chain (see
    /// [`Self::walk_chain`] for what the walk checks and what bounds it).
    fn read_overflow(&self, r: OverflowRef) -> Result<Vec<Value>> {
        // Reserve by what a few pages hold, never by what the reference claims.
        let capacity = usize::try_from(r.len).map_or(0, |len| len.min(64 * DATA_SIZE));
        let mut bytes: Vec<u8> = Vec::with_capacity(capacity);
        self.walk_chain(
            r,
            |id| Ok(self.store.get(id)?),
            |_, data| bytes.extend_from_slice(data),
        )?;
        postcard::from_bytes(&bytes).map_err(|_| {
            CowBtreeError::Storage(PageStoreError::CorruptChain {
                head: r.head,
                page: r.head,
            })
        })
    }

    /// Walk the overflow chain `r` names, handing each link's id and payload to
    /// `visit` once the link has been checked against the reference.
    ///
    /// Each link must be an overflow page carrying a full [`DATA_SIZE`] bytes but the
    /// last, which carries the remainder and ends the chain; that is exactly
    /// [`OverflowRef::page_count`] links. The walk is bounded by the store's
    /// allocation counter, not by the reference: a reference claiming more links than
    /// the store has ever allocated pages is refused before a page is read, a link
    /// must name a page the store has allocated — an id below its `next_page_id`,
    /// which a file store's open refuses past the last page it can address, so two
    /// ids are two pages — and a link the walk has already visited is a cycle. So
    /// the walk visits no page twice and no page the store does not hold, however
    /// the reference and its links were damaged, and what `visit` is handed is at
    /// most one page's payload per page the store holds. Anything else — a missing
    /// link, another kind of page, a wrong length, a chain that goes on past its
    /// last link or ends before it, a loop — is [`PageStoreError::CorruptChain`];
    /// the page checksums already refuse a torn link.
    fn walk_chain(
        &self,
        r: OverflowRef,
        get: impl Fn(PageId) -> Result<Option<Page>>,
        mut visit: impl FnMut(PageId, &[u8]),
    ) -> Result<()> {
        let corrupt = |page: PageId| {
            CowBtreeError::Storage(PageStoreError::CorruptChain { head: r.head, page })
        };
        let allocated = self.store.next_page_id();
        let pages = r.page_count();
        let total = usize::try_from(r.len).map_err(|_| corrupt(r.head))?;
        if pages > allocated {
            return Err(corrupt(r.head));
        }
        let mut visited = HashSet::new();
        let mut remaining = total;
        let mut id = r.head;
        for hop in 1..=pages {
            if id >= allocated || !visited.insert(id) {
                return Err(corrupt(id));
            }
            let Some(Page::Overflow { header, data }) = get(id)? else {
                return Err(corrupt(id));
            };
            let last = hop == pages;
            let want = if last { remaining } else { DATA_SIZE };
            if data.len() != want || last != (header.next_or_reserved == NULL_PAGE_ID) {
                return Err(corrupt(id));
            }
            visit(id, &data);
            remaining -= want;
            id = header.next_or_reserved;
        }
        Ok(())
    }

    /// Scan all entries in a table.
    pub fn scan_table(&self, table_id: u32) -> Result<Vec<(BtreeKey, Vec<Value>)>> {
        let root = self.current_root();
        self.scan_table_at_root(root, table_id)
    }

    /// Scan table using a specific root.
    pub fn scan_table_at_root(
        &self,
        root: PageId,
        table_id: u32,
    ) -> Result<Vec<(BtreeKey, Vec<Value>)>> {
        if root == NULL_PAGE_ID {
            return Ok(Vec::new());
        }

        let start = BtreeKey::table_start(table_id);
        let end = BtreeKey::table_end(table_id);
        self.range_scan_at_root(root, &start, &end)
    }

    /// Range scan using a specific root.
    ///
    /// NOTE: This traverses the tree structure rather than following leaf next
    /// pointers, because `CoW` updates don't maintain the leaf chain (updating
    /// a leaf would require CoW-ing all preceding leaves to update their next
    /// pointers).
    ///
    /// TODO(btree-optimization): Consider removing leaf next pointers entirely
    /// since they're not useful with `CoW`, or implement proper leaf chain
    /// maintenance during `CoW` (expensive: O(n) `CoW` on every update).
    pub fn range_scan_at_root(
        &self,
        root: PageId,
        start: &BtreeKey,
        end: &BtreeKey,
    ) -> Result<Vec<(BtreeKey, Vec<Value>)>> {
        self.range_scan_at_root_limited(root, start, end, None)
    }

    /// Range scan using a specific root, stopping after `limit` entries.
    ///
    /// `limit = None` collects the whole `[start, end)` band (identical to
    /// [`range_scan_at_root`](Self::range_scan_at_root)); `Some(n)` stops the
    /// **ascending** traversal once `n` entries are collected. Because
    /// [`collect_range`](Self::collect_range) walks the tree left-to-right (keys in
    /// ascending order), the first `n` collected are the `n` smallest in range — so
    /// a forward (ascending) range scan with a `LIMIT` early-stops cleanly. A
    /// descending scan can't early-stop here (it wants the *largest* `n`); its
    /// caller collects the full band, reverses, then truncates.
    pub fn range_scan_at_root_limited(
        &self,
        root: PageId,
        start: &BtreeKey,
        end: &BtreeKey,
        limit: Option<usize>,
    ) -> Result<Vec<(BtreeKey, Vec<Value>)>> {
        if root == NULL_PAGE_ID {
            return Ok(Vec::new());
        }
        // A zero limit asks for nothing; skip the traversal entirely.
        if limit == Some(0) {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        self.collect_range(root, start, end, limit, &mut results)?;
        Ok(results)
    }

    /// Probe a secondary index for an exact value, returning matching row IDs.
    ///
    /// Scans the contiguous `[index_start(id, enc) ..= index_end(id, enc)]` band
    /// of index entries (all share the same `(index_id, encoded_value)`, ordered
    /// by `row_id`) at the given root and extracts each entry's `row_id`. The
    /// upper bound is made *exclusive* by appending one byte to the encoded value:
    /// under the order-preserving framing a value is a strict prefix of itself +
    /// any byte, so `enc` sorts before `enc++byte` and every `(id, enc, row_id)`
    /// falls strictly below `index(id, enc++0x00, 0)` — including `row_id` =
    /// `u64::MAX`, which a `[start, end)` scan with an inclusive `index_end` would
    /// drop. Reading at an old `root` yields that snapshot's index state, so the
    /// probe is MVCC-consistent with data reads at the same root.
    pub fn index_scan_at_root(
        &self,
        root: PageId,
        index_id: u32,
        encoded_value: &[u8],
    ) -> Result<Vec<u64>> {
        if root == NULL_PAGE_ID {
            return Ok(Vec::new());
        }

        let start = BtreeKey::index_start(index_id, encoded_value);
        // Exclusive upper bound: `encoded_value` followed by one extra byte sorts
        // strictly after every `(index_id, encoded_value, *)` entry.
        let mut upper = encoded_value.to_vec();
        upper.push(0x00);
        let end_excl = BtreeKey::index(index_id, &upper, 0);

        let entries = self.range_scan_at_root(root, &start, &end_excl)?;
        Ok(entries.into_iter().map(|(key, _)| key.row_id()).collect())
    }

    /// Range-scan a secondary index over pre-built `[start, end)` key bounds,
    /// returning matching row ids in index-key order.
    ///
    /// The range counterpart to [`index_scan_at_root`](Self::index_scan_at_root):
    /// the caller builds `start`/`end` from the value bounds (via
    /// [`BtreeKey::index_range_start`] / [`BtreeKey::index_range_end`]), this walks the
    /// entries between them and projects each entry's `row_id`. `ascending = true`
    /// returns them smallest-key first (the tree's natural order) and, with
    /// `limit = Some(n)`, early-stops at `n`. `ascending = false` returns them
    /// largest-key first: the full band is collected (no descending early-stop today —
    /// a future optimization), reversed, then truncated to `limit`. Reading at an old
    /// `root` yields that snapshot's index state, MVCC-consistent with data reads at the
    /// same root.
    pub fn index_range_scan_at_root(
        &self,
        root: PageId,
        start: &BtreeKey,
        end: &BtreeKey,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<u64>> {
        if root == NULL_PAGE_ID {
            return Ok(Vec::new());
        }

        if ascending {
            // Forward scan early-stops cleanly: the first `n` keys collected are the
            // `n` smallest in range.
            let entries = self.range_scan_at_root_limited(root, start, end, limit)?;
            Ok(entries.into_iter().map(|(key, _)| key.row_id()).collect())
        } else {
            // Descending: collect the whole band (early-stop would yield the wrong
            // `n`), reverse to largest-first, then truncate to the limit.
            let entries = self.range_scan_at_root(root, start, end)?;
            let mut ids: Vec<u64> = entries.into_iter().map(|(key, _)| key.row_id()).collect();
            ids.reverse();
            if let Some(n) = limit {
                ids.truncate(n);
            }
            Ok(ids)
        }
    }

    /// The greatest key in `[start, end)` at `root`, or `None` when the band holds none.
    ///
    /// [`collect_range`](Self::collect_range)'s walk run right to left, stopping at the
    /// first key in range a leaf yields: every subtree to its right was walked first and
    /// held none, so that key is the band's greatest. It reads no value, so an overflow
    /// chain is never followed. A page the walk reaches twice is refused ([`Visited`]).
    ///
    /// # Errors
    ///
    /// A page the store cannot read, or a damaged tree.
    pub fn last_key_in_range_at_root(
        &self,
        root: PageId,
        start: &BtreeKey,
        end: &BtreeKey,
    ) -> Result<Option<BtreeKey>> {
        let mut visited = Visited::new();
        let mut stack = vec![root];
        while let Some(page_id) = stack.pop() {
            if page_id == NULL_PAGE_ID {
                continue;
            }
            visited.enter(page_id)?;
            let page = self
                .store
                .get(page_id)?
                .ok_or(CowBtreeError::PageNotFound(page_id))?;
            match page {
                Page::Leaf { entries, .. } => {
                    if let Some(entry) = entries
                        .iter()
                        .rev()
                        .find(|entry| &entry.key >= start && &entry.key < end)
                    {
                        return Ok(Some(entry.key.clone()));
                    }
                }
                Page::Internal {
                    first_child,
                    entries,
                    ..
                } => {
                    // The children whose key ranges meet [start, end), pushed left to
                    // right so the rightmost is walked first.
                    if first_child != NULL_PAGE_ID
                        && (entries.is_empty() || start < &entries[0].key)
                    {
                        stack.push(first_child);
                    }
                    for (i, entry) in entries.iter().enumerate() {
                        if entry.child == NULL_PAGE_ID {
                            continue;
                        }
                        if let Some(next) = entries.get(i + 1)
                            && next.key <= *start
                        {
                            continue;
                        }
                        if &entry.key >= end {
                            break;
                        }
                        stack.push(entry.child);
                    }
                }
                Page::Free { .. } => {}
                Page::Overflow { .. } => return Err(CowBtreeError::PageNotFound(page_id)),
            }
        }
        Ok(None)
    }

    /// Collect entries in range via tree traversal.
    ///
    /// Walks left-to-right (ascending key order), depth first, from an explicit stack
    /// rather than by recursion, so a tree's depth is not the thread's stack's to
    /// bound. With `limit = Some(n)`, returns as soon as `results` holds `n` entries —
    /// so an ascending `LIMIT` scan stops early without visiting the rest of the band.
    /// `None` collects the whole band. A page the walk reaches twice is refused
    /// ([`Visited`]), so no damaged tree makes the scan loop or emit a subtree twice.
    fn collect_range(
        &self,
        root: PageId,
        start: &BtreeKey,
        end: &BtreeKey,
        limit: Option<usize>,
        results: &mut Vec<(BtreeKey, Vec<Value>)>,
    ) -> Result<()> {
        let mut visited = Visited::new();
        let mut stack = vec![root];
        while let Some(page_id) = stack.pop() {
            if page_id == NULL_PAGE_ID {
                continue;
            }
            // Early-stop: the ascending walk has already collected enough.
            if limit.is_some_and(|n| results.len() >= n) {
                return Ok(());
            }
            visited.enter(page_id)?;

            let page = self
                .store
                .get(page_id)?
                .ok_or(CowBtreeError::PageNotFound(page_id))?;

            match page {
                Page::Leaf { entries, .. } => {
                    for entry in entries.iter() {
                        if &entry.key >= start && &entry.key < end {
                            results.push((entry.key.clone(), self.read_value(&entry.value)?));
                            if limit.is_some_and(|n| results.len() >= n) {
                                return Ok(());
                            }
                        }
                    }
                }
                Page::Internal {
                    first_child,
                    entries,
                    ..
                } => {
                    // The children whose key ranges meet [start, end), left to right;
                    // pushed right to left so the leftmost is walked first.
                    let mut children = Vec::new();
                    // first_child holds the keys below entries[0].key.
                    if first_child != NULL_PAGE_ID
                        && (entries.is_empty() || start < &entries[0].key)
                    {
                        children.push(first_child);
                    }
                    for (i, entry) in entries.iter().enumerate() {
                        if entry.child == NULL_PAGE_ID {
                            continue;
                        }
                        // This child holds keys >= entry.key and < the next entry's
                        // key: skip it if that whole range is before start.
                        if let Some(next) = entries.get(i + 1)
                            && next.key <= *start
                        {
                            continue;
                        }
                        // Every key from here on is >= end.
                        if &entry.key >= end {
                            break;
                        }
                        children.push(entry.child);
                    }
                    stack.extend(children.into_iter().rev());
                }
                Page::Free { .. } => {}
                // A tree pointer never names a chain link: the store is damaged.
                Page::Overflow { .. } => return Err(CowBtreeError::PageNotFound(page_id)),
            }
        }

        Ok(())
    }

    // ========================================================================
    // Write Operations (with CoW)
    // ========================================================================

    /// Insert a key-value pair.
    ///
    /// A thin wrapper over [`apply_batch`](Self::apply_batch) with a one-element
    /// batch: there is a single write path (the batch family), so a lone insert
    /// gets the same flush + seqlock-publish semantics as a multi-row commit.
    pub fn insert(&self, key: BtreeKey, value: Vec<Value>) -> Result<()> {
        let mut batch = BatchChanges::new();
        batch.insert(key, value);
        self.apply_batch(batch)?;
        Ok(())
    }

    /// Delete a key, returning the old value if it existed.
    ///
    /// A thin wrapper over [`apply_batch`](Self::apply_batch): it reads the old
    /// value at the current root first (preserving the historic
    /// `Option<Vec<Value>>` return), then applies a one-element delete batch.
    /// Like the historic single-op path it publishes once per call (one version
    /// bump); `apply_batch` no-ops (no version bump) when the key was absent.
    pub fn delete(&self, key: &BtreeKey) -> Result<Option<Vec<Value>>> {
        let old_value = self.get(key)?;
        if old_value.is_none() {
            return Ok(None);
        }
        let mut batch = BatchChanges::new();
        batch.delete(key.clone());
        self.apply_batch(batch)?;
        Ok(old_value)
    }

    // ========================================================================
    // Batch Operations
    // ========================================================================

    /// Apply a batch of changes atomically.
    ///
    /// This is the primary method for transactional commits. All changes are:
    /// 1. Collected into new CoW pages
    /// 2. Written to the data store (`put`)
    /// 3. Made durable via `flush()` (fsync of the data pages — the first of a
    ///    commit's two fsyncs)
    /// 4. Committed by the atomic root/version swap
    /// 5. Old pages registered for GC
    ///
    /// The second fsync — the meta-slot flip that is the durable commit point —
    /// is performed by `save_state`, which the layer that owns the schema (the
    /// wrapper) drives with the full metadata right after this returns.
    ///
    /// A page write or `fsync` the device refuses fails the store
    /// ([`PageStoreError::WriteFailed`]); [`Self::apply_discardable_batch`] is the one
    /// form that may leave it serving.
    ///
    /// Returns the number of operations applied.
    pub fn apply_batch(&self, changes: BatchChanges) -> Result<usize> {
        self.apply_batch_settled(changes, false)
    }

    /// [`Self::apply_batch`] for a batch its caller can drop whole: a page write or
    /// `fsync` the device refuses answers [`PageStoreError::WriteRefused`] and leaves
    /// the store serving — the batch published nothing and is gone, and readers see the
    /// tree as it was before it.
    ///
    /// For a caller that keeps no other effect of the batch: the wrapper's commit of a
    /// transaction's staged rows, when the transaction changed nothing else that its
    /// host's rollback does not take back.
    ///
    /// # Errors
    ///
    /// Every error [`Self::apply_batch`] returns, and `WriteRefused` as above.
    pub fn apply_discardable_batch(&self, changes: BatchChanges) -> Result<usize> {
        self.apply_batch_settled(changes, true)
    }

    /// [`Self::apply_batch`]'s body, a refused write settled by
    /// [`Self::settle_refusal`] with `discardable`.
    fn apply_batch_settled(&self, changes: BatchChanges, discardable: bool) -> Result<usize> {
        if changes.is_empty() {
            return Ok(0);
        }

        let _guard = self.write_lock.lock();

        // Decide every insert's placement up front, refusing an entry no page can
        // hold — a lone entry can't be split, so it must fit one page (its value out
        // of line if need be), and its key must fit an internal page as a separator
        // should a split promote it. With both true for every entry, every page the
        // split builds fits (`page::split_points`). Validating before touching the
        // working set keeps the batch atomic (no pages written, no root published).
        let placements = changes
            .inserts
            .iter()
            .map(|(key, value)| placement(key, value))
            .collect::<Result<Vec<Placement>>>()?;

        let old_root = self.current_root();

        // Copy-on-first-write working set: each pre-batch page is copied at most
        // once for the whole batch; pages created within the batch are mutated in
        // place. This collapses the per-op root-to-leaf re-copy (≈ tree depth per
        // row) down to the set of pages actually touched by the batch.
        let mut ws = BatchWorkingSet::new();

        // From here every refusal hands the batch's fresh page ids back to the free
        // list: no root has reached them, and no root will, since the new one is
        // published only once the pages are written and flushed.
        let (working_root, ops_count) =
            match self.build_batch(&mut ws, old_root, changes, placements) {
                Ok(built) => built,
                Err(e) => {
                    self.store.free_pages(ws.created.into_iter().collect());
                    return Err(e);
                }
            };

        if working_root == old_root && ws.pages.is_empty() {
            // No actual changes
            self.store.free_pages(ws.created.into_iter().collect());
            return Ok(0);
        }

        // Write each unique dirtied page exactly once — but only once every one of
        // them is known to fit, which `put_batch` checks before it stores any. The
        // checks above make that true of every page the split builds; this is the
        // belt that keeps a violation from ever reaching a store half-written.
        let written = match self.store.put_batch(ws.pages.into_values().collect()) {
            Ok(()) => Ok(()),
            Err(
                err @ (PageStoreError::Overfull { .. }
                | PageStoreError::WriteRefused(_)
                | PageStoreError::WriteFailed(_)
                | PageStoreError::StoreFailed(_)),
            ) => Err(CowBtreeError::Storage(err)),
            Err(e) => Err(CowBtreeError::Internal(format!(
                "batch page put failed: {e}"
            ))),
        };
        // Make the freshly written data pages durable. This is the first of a
        // commit's two fsyncs; the second is the meta-slot flip in `save_state`,
        // which the layer that owns the schema (the wrapper) drives with the
        // FULL metadata right after this returns. We deliberately do NOT
        // fabricate metadata here — `apply_batch` has no schema/table-mapping
        // context, so synthesizing it would wipe the catalog on every commit.
        let flushed = written.and_then(|()| self.store.flush().map_err(CowBtreeError::Storage));
        if let Err(e) = flushed {
            self.store.free_pages(ws.created.into_iter().collect());
            return Err(self.settle_refusal(e, discardable));
        }
        let all_old_pages: Vec<PageId> = ws.superseded;

        // Calculate next version
        let new_version = self.version.load(Ordering::SeqCst) + 1;

        // Atomic root + version swap (the in-memory commit point; the durable
        // commit point is the meta-slot flip in the subsequent `save_state`).
        // Seqlock-protected so lock-free readers never see a torn pair.
        self.publish_root(working_root, new_version);

        // Register old pages for GC. They are not free, but no root from `new_version`
        // on reaches them, so a save of such a version logs them (`save_state`).
        if !all_old_pages.is_empty() {
            let freed_version = new_version.saturating_sub(1);
            let freed_count = all_old_pages.len();
            self.pending_gc.lock().push((all_old_pages, freed_version));
            // Keep the lock-free page-count shadow in step.
            self.pending_gc_pages
                .fetch_add(freed_count, Ordering::Relaxed);
        }

        Ok(ops_count)
    }

    /// Apply a batch's deletes, then its inserts, to the working set, returning the
    /// batch tree's root and how many operations changed it.
    ///
    /// Deletes go first. When one row change touches the same key with both a delete
    /// (old value) and an insert (new value) — which happens for an UPDATE that leaves
    /// an indexed column unchanged, so the index entry's key is identical before and
    /// after — the insert must win (the entry survives). Inserts-first would let the
    /// trailing delete wipe a still-live entry, desyncing the index from the data.
    ///
    /// Each out-of-line value's overflow chain is written into the working set before
    /// its leaf: its pages are written, fsynced and published with the leaf that
    /// references them, and freed like any page the batch supersedes.
    fn build_batch(
        &self,
        ws: &mut BatchWorkingSet,
        old_root: PageId,
        changes: BatchChanges,
        placements: Vec<Placement>,
    ) -> Result<(PageId, usize)> {
        let mut working_root = old_root;
        let mut ops_count = 0;
        for key in changes.deletes {
            if let Some(new_root) = self.apply_delete_ws(ws, working_root, &key)? {
                working_root = new_root;
                ops_count += 1;
            }
        }
        for ((key, value), placement) in changes.inserts.into_iter().zip(placements) {
            let value = match placement {
                Placement::Inline => LeafValue::Inline(value),
                Placement::OutOfLine => LeafValue::Overflow(self.write_chain(ws, &value)?),
            };
            working_root = self.apply_insert_ws(ws, working_root, key, value)?;
            ops_count += 1;
        }
        Ok((working_root, ops_count))
    }

    /// Write `value`'s postcard encoding into a fresh overflow chain in the working
    /// set, returning the reference a leaf entry holds for it.
    fn write_chain(&self, ws: &mut BatchWorkingSet, value: &[Value]) -> Result<OverflowRef> {
        let bytes = encode_value(value)?;
        let ids: Vec<PageId> = bytes
            .chunks(DATA_SIZE)
            .map(|_| self.store.allocate())
            .collect();
        for (i, chunk) in bytes.chunks(DATA_SIZE).enumerate() {
            let next = ids.get(i + 1).copied().unwrap_or(NULL_PAGE_ID);
            ws.insert_created(Page::new_overflow(ids[i], chunk.to_vec(), next));
        }
        Ok(OverflowRef {
            head: ids.first().copied().unwrap_or(NULL_PAGE_ID),
            len: bytes.len() as u64,
        })
    }

    /// Release the overflow chain of a value the batch deletes or replaces.
    ///
    /// A link that existed before the batch is superseded — freed once no snapshot
    /// can reach the version that references it, by the same rule that frees the
    /// leaf — and a link this batch created is dropped outright, never written. The
    /// chain is walked as a read walks it, so a link the walk revisits (one chain
    /// naming a page twice) is refused, and so is a pre-batch link this batch has
    /// already released (two entries naming one chain): those two refusals are what
    /// keep a damaged chain's page from reaching the free list twice within a batch.
    ///
    /// What leaves a refused release harmless is `apply_batch`, not the order here:
    /// its error returns before the working set is stored, and the superseded ids go
    /// with the working set. The one release that reaches past the working set is a
    /// created link's (`discard_created` returns its id to the free list at once),
    /// and that page was never written and no root reaches it.
    fn release_chain(&self, ws: &mut BatchWorkingSet, r: OverflowRef) -> Result<()> {
        let mut links = Vec::new();
        self.walk_chain(
            r,
            |id| match ws.pages.get(&id) {
                Some(page) => Ok(Some(page.clone())),
                None => Ok(self.store.get(id)?),
            },
            |id, _| links.push(id),
        )?;
        for id in links {
            if ws.is_created(id) {
                ws.discard_created(self.store.as_ref(), id);
            } else if ws.released.insert(id) {
                ws.supersede(id);
            } else {
                return Err(CowBtreeError::Storage(PageStoreError::CorruptChain {
                    head: r.head,
                    page: id,
                }));
            }
        }
        Ok(())
    }

    /// Apply an insert against the batch working set.
    ///
    /// Returns the new root for the (in-progress) batch tree. Pages are dirtied
    /// in `ws` (copied at most once per batch, then mutated in place) rather than
    /// written to the store here; the store write happens once per unique page at
    /// the end of `apply_batch`.
    fn apply_insert_ws(
        &self,
        ws: &mut BatchWorkingSet,
        current_root: PageId,
        key: BtreeKey,
        value: LeafValue,
    ) -> Result<PageId> {
        if current_root == NULL_PAGE_ID {
            // Create first leaf as root.
            let page_id = self.store.allocate();
            let mut page = Page::new_leaf(page_id);
            if let Page::Leaf { entries, header } = &mut page {
                Arc::make_mut(entries).push(LeafEntry { key, value });
                header.count = 1;
            }
            ws.insert_created(page);
            return Ok(page_id);
        }

        // Find path from root to leaf within the working set.
        let (path, leaf_id) = self.find_leaf_with_path_ws(ws, current_root, &key)?;

        // Modify leaf (in place if created in this batch, else copy-on-first-write).
        let (new_leaf_id, split_info) = self.cow_insert_leaf_ws(ws, leaf_id, key, value)?;

        // Propagate changes up the path.
        self.cow_propagate_up_ws(ws, path, new_leaf_id, split_info)
    }

    /// Apply a delete against the batch working set.
    ///
    /// Returns `Some(new_root)` if the key existed, `None` otherwise.
    fn apply_delete_ws(
        &self,
        ws: &mut BatchWorkingSet,
        current_root: PageId,
        key: &BtreeKey,
    ) -> Result<Option<PageId>> {
        if current_root == NULL_PAGE_ID {
            return Ok(None);
        }

        let (path, leaf_id) = self.find_leaf_with_path_ws(ws, current_root, key)?;

        let (deleted, new_leaf_id) = self.cow_delete_leaf_ws(ws, leaf_id, key)?;
        if !deleted {
            return Ok(None);
        }

        // Propagate changes up (simplified: no rebalancing).
        Ok(Some(self.cow_propagate_up_ws(
            ws,
            path,
            new_leaf_id,
            Vec::new(),
        )?))
    }

    /// Find leaf page with full path from a specific root, reading through the
    /// working set so in-batch modifications are visible to later ops.
    fn find_leaf_with_path_ws(
        &self,
        ws: &BatchWorkingSet,
        root: PageId,
        key: &BtreeKey,
    ) -> Result<(Vec<PathElement>, PageId)> {
        let mut path = Vec::new();
        let mut page_id = root;
        let mut visited = Visited::new();

        loop {
            visited.enter(page_id)?;
            let page = ws.get(self.store.as_ref(), page_id)?;

            match page {
                Page::Leaf { .. } => return Ok((path, page_id)),
                Page::Internal {
                    first_child,
                    entries,
                    ..
                } => {
                    let mut child = first_child;
                    let mut child_slot = ChildSlot::First;

                    for (i, entry) in entries.iter().enumerate() {
                        if key < &entry.key {
                            break;
                        }
                        child = entry.child;
                        child_slot = ChildSlot::Entry(i);
                    }

                    path.push(PathElement {
                        page_id,
                        child_slot,
                    });
                    page_id = child;
                }
                Page::Free { .. } | Page::Overflow { .. } => {
                    return Err(CowBtreeError::PageNotFound(page_id));
                }
            }
        }
    }

    /// Write a dirtied page into `ws`, choosing its id by the CoW immutability rule:
    /// reuse `old_id` in place when it was **created in this batch** (mutate the fresh
    /// page), otherwise allocate a new id and mark `old_id` superseded (so an older
    /// root keeps seeing the pre-mutation page). `build(new_id)` constructs the page
    /// with its id-stamped header; the new id is returned.
    ///
    /// The single home for the reuse-or-allocate → supersede → `insert_created`
    /// bookkeeping every leaf/internal rewrite repeats — the MVCC-immutability
    /// invariant lives here once instead of in five hand-copied blocks.
    fn write_dirtied(
        &self,
        ws: &mut BatchWorkingSet,
        old_id: PageId,
        build: impl FnOnce(PageId) -> Page,
    ) -> PageId {
        let created = ws.is_created(old_id);
        let new_id = if created {
            old_id
        } else {
            self.store.allocate()
        };
        let page = build(new_id);
        if !created {
            ws.supersede(old_id);
        }
        ws.insert_created(page);
        new_id
    }

    /// Grow a fresh root over the splits that bubbled past the old root: a new
    /// internal node with `first_child` on the left and the separators as its
    /// entries. If they do not fit one page, the new node is split too and the loop
    /// grows another level, until a root fits. Each round at least halves the
    /// separators (a greedy piece holds at least one entry before each promoted
    /// one), so it ends. Inserted into `ws`; the new root id is returned.
    fn grow_root_ws(
        &self,
        ws: &mut BatchWorkingSet,
        mut first_child: PageId,
        mut splits: CowSplit,
    ) -> PageId {
        loop {
            let mut entries: Vec<InternalEntry> = std::mem::take(&mut splits)
                .into_iter()
                .map(|(key, child)| InternalEntry { key, child })
                .collect();
            if !internal_entries_fit(&entries) {
                splits = self.split_internal_ws(ws, &mut entries);
            }
            let root_id = self.store.allocate();
            let mut header = PageHeader::new(root_id, PageFlags::Internal);
            header.count = entries.len() as u16;
            ws.insert_created(Page::Internal {
                header,
                first_child,
                entries: Arc::new(entries),
            });
            if splits.is_empty() {
                return root_id;
            }
            first_child = root_id;
        }
    }

    /// Split an internal node's over-budget `entries` in place: they keep the
    /// leftmost piece, and each further piece becomes a fresh internal page whose
    /// `first_child` is the child of the separator promoted over it. Returns those
    /// pages with their separators, left to right, for the parent.
    fn split_internal_ws(
        &self,
        ws: &mut BatchWorkingSet,
        entries: &mut Vec<InternalEntry>,
    ) -> CowSplit {
        let sizes: Vec<usize> = entries
            .iter()
            .map(|e| internal_entry_bytes(&e.key))
            .collect();
        let cuts = split_points(&sizes, 8, true);
        let mut splits: CowSplit = Vec::with_capacity(cuts.len());
        // Peel pieces off the right end, so the earlier cut indices stay valid.
        for &cut in cuts.iter().rev() {
            let mut tail: Vec<InternalEntry> = entries.drain(cut..).collect();
            let promoted = tail.remove(0);
            let id = self.store.allocate();
            let mut header = PageHeader::new(id, PageFlags::Internal);
            header.count = tail.len() as u16;
            ws.insert_created(Page::Internal {
                header,
                first_child: promoted.child,
                entries: Arc::new(tail),
            });
            splits.push((promoted.key, id));
        }
        splits.reverse();
        splits
    }

    /// CoW insert into leaf, writing pages to storage and collecting them.
    ///
    /// Pages are written to storage immediately so subsequent batch operations can find them.
    /// They're also collected for WAL writing.
    ///
    /// Returns `(new_leaf_id, split_info)`. The leaf is dirtied in `ws`: mutated
    /// in place if it was already created in this batch, otherwise copied once.
    fn cow_insert_leaf_ws(
        &self,
        ws: &mut BatchWorkingSet,
        leaf_id: PageId,
        key: BtreeKey,
        value: LeafValue,
    ) -> Result<(PageId, CowSplit)> {
        let leaf = ws.take_for_write(self.store.as_ref(), leaf_id)?;

        let Page::Leaf {
            mut entries,
            header,
        } = leaf
        else {
            return Err(CowBtreeError::PageNotFound(leaf_id));
        };

        // Copy-on-shared: if any old root / cache still references this entry
        // vector (refcount > 1) `make_mut` deep-copies it, so old snapshots keep
        // seeing the pre-mutation page; if this is the sole owner it mutates in
        // place. Descent never reaches here, so it never pays this copy.
        let entries_vec = Arc::make_mut(&mut entries);

        // Find insertion point. A replaced value's overflow chain, if it had one, is
        // released once the leaf is back in hand.
        let replaced = match entries_vec.binary_search_by(|e| e.key.cmp(&key)) {
            Ok(idx) => Some(std::mem::replace(&mut entries_vec[idx].value, value)),
            Err(idx) => {
                entries_vec.insert(idx, LeafEntry { key, value });
                None
            }
        };
        if let Some(LeafValue::Overflow(r)) = replaced {
            self.release_chain(ws, r)?;
        }
        let entries_vec = Arc::make_mut(&mut entries);

        // Split when the serialized leaf would exceed the page byte budget, into as
        // many pieces as it takes for each to fit (`page::split_points`). A lone
        // entry can't be split further, so keep at least one.
        if entries_vec.len() > 1 && !leaf_entries_fit(entries_vec) {
            let sizes: Vec<usize> = entries_vec
                .iter()
                .map(|e| leaf_entry_bytes(&e.key, &e.value))
                .collect();
            let cuts = split_points(&sizes, 0, false);
            // Peel pieces off the right end, so the earlier cut indices stay valid.
            let mut pieces: Vec<Vec<LeafEntry>> = cuts
                .iter()
                .rev()
                .map(|&cut| entries_vec.drain(cut..).collect())
                .collect();
            pieces.reverse();

            // Right pieces are fresh pages, each linked to the next one (the last to
            // the original leaf's successor).
            let mut next = header.next_or_reserved;
            let mut splits: CowSplit = Vec::with_capacity(pieces.len());
            for piece in pieces.into_iter().rev() {
                let id = self.store.allocate();
                let mut piece_header = PageHeader::new(id, PageFlags::Leaf);
                piece_header.count = piece.len() as u16;
                piece_header.next_or_reserved = next;
                next = id;
                splits.push((piece[0].key.clone(), id));
                ws.insert_created(Page::Leaf {
                    header: piece_header,
                    entries: Arc::new(piece),
                });
            }
            splits.reverse();
            let first_right = next;

            // Left leaf reuses the original page id if it was created in this
            // batch (mutate in place); otherwise copy-on-first-write.
            let new_left_id = self.write_dirtied(ws, leaf_id, |id| {
                let mut new_left_header = PageHeader::new(id, PageFlags::Leaf);
                new_left_header.count = entries.len() as u16;
                new_left_header.next_or_reserved = first_right;
                Page::Leaf {
                    header: new_left_header,
                    entries,
                }
            });

            return Ok((new_left_id, splits));
        }

        // No split: write back the modified leaf.
        let new_leaf_id = self.write_dirtied(ws, leaf_id, |id| {
            let mut new_header = PageHeader::new(id, PageFlags::Leaf);
            new_header.count = entries.len() as u16;
            new_header.next_or_reserved = header.next_or_reserved;
            Page::Leaf {
                header: new_header,
                entries,
            }
        });

        Ok((new_leaf_id, Vec::new()))
    }

    /// CoW delete from leaf against the working set.
    ///
    /// Returns `(deleted, new_leaf_id)`. On a hit the leaf is dirtied in `ws`
    /// (in place if created in this batch, else copied once); on a miss nothing
    /// is dirtied and the original id is returned.
    fn cow_delete_leaf_ws(
        &self,
        ws: &mut BatchWorkingSet,
        leaf_id: PageId,
        key: &BtreeKey,
    ) -> Result<(bool, PageId)> {
        let leaf = ws.take_for_write(self.store.as_ref(), leaf_id)?;

        let Page::Leaf {
            mut entries,
            header,
        } = leaf
        else {
            return Err(CowBtreeError::PageNotFound(leaf_id));
        };

        // Copy-on-shared (see `cow_insert_leaf_ws`): only deep-copies the entry
        // vector if an old root / cache still shares it, so MVCC snapshots stay
        // immutable.
        if let Ok(idx) = entries.binary_search_by(|e| e.key.cmp(key)) {
            let removed = Arc::make_mut(&mut entries).remove(idx);
            if let LeafValue::Overflow(r) = removed.value {
                self.release_chain(ws, r)?;
            }

            let new_leaf_id = self.write_dirtied(ws, leaf_id, |id| {
                let mut new_header = PageHeader::new(id, PageFlags::Leaf);
                new_header.count = entries.len() as u16;
                new_header.next_or_reserved = header.next_or_reserved;
                Page::Leaf {
                    header: new_header,
                    entries,
                }
            });

            Ok((true, new_leaf_id))
        } else {
            // Delete miss: nothing changed. `take_for_write` *removed* a created
            // page from `pages`, so restore it unchanged (a pre-batch page was
            // cloned, not removed — nothing to put back).
            if ws.is_created(leaf_id) {
                ws.insert_created(Page::Leaf { header, entries });
            }
            Ok((false, leaf_id))
        }
    }

    /// Propagate CoW changes up the path against the working set.
    ///
    /// Each internal node on the path is dirtied at most once per batch: mutated
    /// in place if it was already created in this batch, otherwise copied once
    /// (the original id is recorded as superseded). Returns the new root id.
    fn cow_propagate_up_ws(
        &self,
        ws: &mut BatchWorkingSet,
        path: Vec<PathElement>,
        new_child: PageId,
        split_info: CowSplit,
    ) -> Result<PageId> {
        let mut current_new_child = new_child;
        let mut current_split = split_info;

        // Process path from leaf to root.
        for path_elem in path.into_iter().rev() {
            let parent_id = path_elem.page_id;
            let parent = ws.take_for_write(self.store.as_ref(), parent_id)?;

            let (first_child, mut entries) = match parent {
                Page::Internal {
                    first_child,
                    entries,
                    ..
                } => (first_child, entries),
                other => {
                    // Non-internal on an internal path (shouldn't happen): restore
                    // the page untouched (if `take_for_write` removed a created
                    // one) and leave the child pointer as-is.
                    if ws.is_created(parent_id) {
                        ws.insert_created(other);
                    }
                    continue;
                }
            };
            let mut first_child = first_child;

            // Copy-on-shared (see `cow_insert_leaf_ws`): deep-copies the entry
            // vector only if an old root / cache still shares it, keeping MVCC
            // snapshots immutable. Descent never reaches here.
            let entries_vec = Arc::make_mut(&mut entries);

            // Update child pointer to the rewritten child.
            match path_elem.child_slot {
                ChildSlot::First => first_child = current_new_child,
                ChildSlot::Entry(idx) => entries_vec[idx].child = current_new_child,
            }

            // Handle the splits bubbling up from below: every separator lands in
            // the gap the rewritten child occupies, in key order.
            if !current_split.is_empty() {
                for (split_key, right_child) in std::mem::take(&mut current_split) {
                    let insert_idx = entries_vec
                        .binary_search_by(|e| e.key.cmp(&split_key))
                        .unwrap_or_else(|i| i);
                    entries_vec.insert(
                        insert_idx,
                        InternalEntry {
                            key: split_key,
                            child: right_child,
                        },
                    );
                }

                // Split when the serialized internal node would exceed the budget.
                if !internal_entries_fit(entries_vec) {
                    current_split = self.split_internal_ws(ws, entries_vec);

                    // Left internal node reuses the parent id if it was created
                    // in this batch; otherwise copy-on-first-write.
                    current_new_child = self.write_dirtied(ws, parent_id, |id| {
                        let mut new_left_header = PageHeader::new(id, PageFlags::Internal);
                        new_left_header.count = entries.len() as u16;
                        Page::Internal {
                            header: new_left_header,
                            first_child,
                            entries,
                        }
                    });
                    continue;
                }
            }

            // No split: write back the rewritten internal node.
            let new_internal_id = self.write_dirtied(ws, parent_id, |id| {
                let mut new_header = PageHeader::new(id, PageFlags::Internal);
                new_header.count = entries.len() as u16;
                Page::Internal {
                    header: new_header,
                    first_child,
                    entries,
                }
            });

            current_new_child = new_internal_id;
        }

        // If splits still bubble past the old root (or the modified page was the
        // root), grow a fresh root over them.
        if !current_split.is_empty() {
            return Ok(self.grow_root_ws(ws, current_new_child, current_split));
        }

        Ok(current_new_child)
    }

    // ========================================================================
    // Internal Helpers
    // ========================================================================

    /// Find leaf page starting from a given root.
    fn find_leaf_from(&self, root: PageId, key: &BtreeKey) -> Result<PageId> {
        let mut page_id = root;
        let mut visited = Visited::new();

        loop {
            visited.enter(page_id)?;
            let page = self
                .store
                .get(page_id)?
                .ok_or(CowBtreeError::PageNotFound(page_id))?;

            match page {
                Page::Leaf { .. } => return Ok(page_id),
                Page::Internal {
                    first_child,
                    entries,
                    ..
                } => {
                    let mut child = first_child;
                    for entry in entries.iter() {
                        if key < &entry.key {
                            break;
                        }
                        child = entry.child;
                    }
                    page_id = child;
                }
                Page::Free { .. } | Page::Overflow { .. } => {
                    return Err(CowBtreeError::PageNotFound(page_id));
                }
            }
        }
    }

    /// Get statistics.
    pub fn stats(&self) -> CowBtreeStats {
        let (pending_gc_batches, pending_gc_pages) = self.pending_gc_count();
        CowBtreeStats {
            root: self.current_root(),
            version: self.current_version(),
            page_count: self.store.page_count(),
            active_snapshots: self
                .active_snapshots
                .read()
                .values()
                .filter(|count| count.load(Ordering::SeqCst) > 0)
                .count(),
            pending_gc_batches,
            pending_gc_pages,
            free_list_size: self.store.free_list_size(),
        }
    }
}

impl Default for CowBtree {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CowBtree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CowBtree")
            .field("root", &self.current_root())
            .field("version", &self.current_version())
            .field("page_count", &self.store.page_count())
            .field("vacuum_threshold", &self.vacuum_threshold)
            .finish_non_exhaustive()
    }
}

/// A snapshot of the B+tree at a specific version.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub version: Version,
    pub root: PageId,
}

/// Statistics for `CoW` B+tree.
#[derive(Debug, Clone)]
pub struct CowBtreeStats {
    pub root: PageId,
    pub version: Version,
    pub page_count: usize,
    pub active_snapshots: usize,
    /// Number of page batches pending garbage collection.
    pub pending_gc_batches: usize,
    /// Total number of pages pending garbage collection.
    pub pending_gc_pages: usize,
    /// Number of pages in the free list available for reuse.
    pub free_list_size: usize,
}

/// Vacuum result statistics.
#[derive(Debug, Clone, Default)]
pub struct VacuumResult {
    /// Number of pages freed in this vacuum run.
    pub pages_freed: usize,
    /// Number of pages still pending (waiting for snapshots to release).
    pub pages_pending: usize,
    /// Minimum active snapshot version (None if no active snapshots).
    pub min_active_version: Option<Version>,
}

#[cfg(test)]
mod tests;
