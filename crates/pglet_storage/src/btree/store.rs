//! Page store abstraction for B+tree storage backends.
//!
//! This module defines the `PageStore` trait which abstracts over different
//! storage backends (in-memory and the single-file double-meta store).

use super::page::{Page, PageId};

/// Errors that can occur during page store operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageStoreError {
    /// Page not found in storage.
    PageNotFound(PageId),
    /// A stored page failed its checksum (torn or bit-rotted bytes on disk) or does
    /// not decode, or a read named an id past every page the store can address.
    /// Distinct from `PageNotFound` so corruption is a loud, catchable error
    /// rather than indistinguishable from a genuinely-missing page.
    Corruption(PageId),
    /// A page whose serialized form would exceed one page. Every page the tree
    /// builds is split to fit, so this is a belt: whatever escapes the split and the
    /// up-front entry checks becomes this error, refused before a byte is written,
    /// rather than a write past the page buffer.
    Overfull {
        /// The page that does not fit.
        page_id: PageId,
        /// Its serialized size, header included.
        bytes: usize,
    },
    /// An overflow chain that does not read back as the reference to it says: a
    /// link missing, of the wrong kind or length, or linked on past its end. A
    /// structural fault the page checksums cannot see, refused rather than read
    /// past or followed without bound.
    CorruptChain {
        /// The chain's first page, as the leaf entry names it.
        head: PageId,
        /// The link where the chain stopped agreeing with its reference.
        page: PageId,
    },
    /// A tree walk reached one page twice: a page names an ancestor, or two pages
    /// name one child, which no honest tree does. Refused where the walk meets it,
    /// so a damaged tree cannot make a read loop or a write grow without bound.
    CorruptTree {
        /// The page the walk reached a second time.
        page: PageId,
    },
    /// I/O error during storage operation.
    Io(String),
    /// A page write or `fsync` the device refused, which has **not** failed the store.
    /// A store answers every refused tree-page write (`put`) and data `fsync` (`flush`)
    /// this way — a refusal inside `save_state` comes back [`Self::WriteFailed`], or
    /// [`Self::SaveUnconfirmed`] at the slot's own write or `fsync` — and leaves
    /// the decision to the tree: dropping the refused batch loses nothing a reader saw
    /// only when its caller keeps no other effect of it
    /// (`CowBtree::apply_discardable_batch`); otherwise the tree fails the store
    /// ([`Self::WriteFailed`]).
    WriteRefused(DeviceError),
    /// A write or `fsync` the device refused, which **has** failed the store: the state
    /// readers can see may hold what the durable state does not, and the durable state
    /// holds nothing of what was being written, since no meta slot naming it was
    /// written. The store records it ([`WriteFailure`]) and refuses everything after it.
    WriteFailed(DeviceError),
    /// A save the device refused at or after its meta-slot write — the slot write, or
    /// the `fsync` that makes it the commit point — which has failed the store as
    /// [`Self::WriteFailed`] does, and whose version may be durable all the same: the
    /// slot may have landed, and a reopen adopts it if it did.
    SaveUnconfirmed(DeviceError),
    /// An operation refused because an earlier write failed ([`Self::WriteFailed`] or
    /// [`Self::SaveUnconfirmed`]); carries that failure's reason. Only a reopen clears it.
    StoreFailed(String),
    /// Storage is closed or invalid.
    Closed,
}

impl std::fmt::Display for PageStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PageNotFound(id) => write!(f, "page not found: {id}"),
            Self::Corruption(id) => write!(
                f,
                "page {id} is corrupt: it fails its checksum, does not decode, or is past every page the store addresses"
            ),
            Self::Overfull { page_id, bytes } => write!(
                f,
                "page {page_id} would serialize to {bytes} bytes, over the {} a page holds",
                super::page::PAGE_SIZE
            ),
            Self::CorruptChain { head, page } => write!(
                f,
                "overflow chain from page {head} is damaged at page {page} (corrupt store)"
            ),
            Self::CorruptTree { page } => write!(
                f,
                "B+tree page {page} is reached twice by one walk (corrupt store)"
            ),
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
            Self::WriteRefused(e) => write!(f, "write refused: {e}"),
            Self::WriteFailed(e) => write!(f, "write failed: {e}"),
            Self::SaveUnconfirmed(e) => write!(f, "save unconfirmed: {e}"),
            Self::StoreFailed(msg) => {
                write!(
                    f,
                    "the store refuses writes and reads since a write failed: {msg}"
                )
            }
            Self::Closed => write!(f, "storage is closed"),
        }
    }
}

impl std::error::Error for PageStoreError {}

/// Result type for page store operations.
pub type PageStoreResult<T> = Result<T, PageStoreError>;

/// A write or `fsync` the device refused.
///
/// Carries the OS error's text and whether the device was out of space, which a client
/// is told apart — `53100` rather than `58030`, as 18.6's `errcode_for_file_access`
/// splits `ENOSPC` from `EIO`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceError {
    /// The OS error's text.
    pub reason: String,
    /// Whether the error was `ENOSPC` (`std::io::ErrorKind::StorageFull`).
    pub disk_full: bool,
}

impl DeviceError {
    /// The device error an I/O error carries.
    #[must_use]
    pub fn from_io(e: &std::io::Error) -> Self {
        Self {
            reason: e.to_string(),
            disk_full: e.kind() == std::io::ErrorKind::StorageFull,
        }
    }
}

impl std::fmt::Display for DeviceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

/// The write or `fsync` that failed a store, once one has: a refused save, or a refused
/// page write or `fsync` its tree could not drop ([`PageStoreError::WriteFailed`]).
///
/// Shared by handle, so a host can refuse work without taking the lock its storage
/// sits behind. Set once and never cleared: the store's in-memory state may then hold
/// a commit its durable state does not, and only a reopen — which reads the durable
/// state alone — ends that.
#[derive(Debug, Clone, Default)]
pub struct WriteFailure(std::sync::Arc<std::sync::OnceLock<String>>);

impl WriteFailure {
    /// The failure's reason, or `None` while every write has completed.
    #[must_use]
    pub fn get(&self) -> Option<&str> {
        self.0.get().map(String::as_str)
    }

    /// Record `reason` unless a failure is already recorded (the first one stands).
    pub fn record(&self, reason: String) {
        let _ = self.0.set(reason);
    }

    /// `Err(StoreFailed)` once a failure is recorded.
    ///
    /// # Errors
    ///
    /// [`PageStoreError::StoreFailed`] carrying the recorded reason.
    pub fn check(&self) -> PageStoreResult<()> {
        self.get().map_or(Ok(()), |reason| {
            Err(PageStoreError::StoreFailed(reason.to_string()))
        })
    }
}

/// Trait for page storage backends.
///
/// Implementations provide durable or ephemeral storage for B+tree pages.
/// The trait is designed to be simple enough for both sync (native) and
/// async-wrapped (WASM/OPFS) implementations.
pub trait PageStore: Send + Sync {
    /// Allocate a new page ID.
    ///
    /// Returns a unique page ID. The implementation may reuse IDs from
    /// the free list or allocate fresh IDs.
    fn allocate(&self) -> PageId;

    /// Get a page by ID.
    ///
    /// Returns `Ok(None)` if the page doesn't exist or has been freed, and
    /// `Err(PageStoreError::Corruption(_))` if a stored page fails its
    /// checksum or does not decode, or the id is past every page the store can
    /// address — corruption is a loud, catchable error, never silently a
    /// missing page.
    fn get(&self, page_id: PageId) -> PageStoreResult<Option<Page>>;

    /// Store a page.
    ///
    /// The page is identified by its embedded `page_id`. After this call,
    /// `get(page.page_id())` should return a clone of this page.
    fn put(&self, page: Page) -> PageStoreResult<()>;

    /// Store every page of one commit, or refuse a page that would not fit before
    /// any of them is stored.
    ///
    /// The contract `CowBtree::apply_batch` relies on: an
    /// [`PageStoreError::Overfull`] from this call means **no** page of the batch was
    /// stored, so the batch's fresh page ids can go straight back to the free list.
    /// Any other error may leave some of the pages stored — unreferenced, since the
    /// root that would reach them is published only after this returns. The default
    /// checks every page ([`Page::check_fits`]) and then `put`s each; a store whose
    /// `put` repeats that check overrides it to check once.
    fn put_batch(&self, pages: Vec<Page>) -> PageStoreResult<()> {
        for page in &pages {
            page.check_fits()?;
        }
        for page in pages {
            self.put(page)?;
        }
        Ok(())
    }

    /// Mark pages as available for reuse.
    ///
    /// These page IDs will be returned by future `allocate()` calls.
    fn free_pages(&self, page_ids: Vec<PageId>);

    /// The store's record of a write it could not complete, for a store that has
    /// durable state to lose — `None` for one that persists nothing.
    fn write_failure(&self) -> Option<&WriteFailure> {
        None
    }

    /// The version of the tree the store's durable state holds — the one a reopen
    /// adopts — or `None` for a store that persists nothing. Garbage collection frees
    /// no page that version reaches, whatever the snapshots say, since a crash before
    /// the next durable commit reopens it.
    fn durable_version(&self) -> Option<u64> {
        None
    }

    /// Get the total number of pages in storage.
    fn page_count(&self) -> usize;

    /// Get the number of pages in the free list.
    fn free_list_size(&self) -> usize;

    /// Get the next page ID that would be allocated (for metadata persistence).
    fn next_page_id(&self) -> PageId;

    /// Get all free page IDs (for metadata persistence).
    fn free_list(&self) -> Vec<PageId>;

    /// Flush any buffered writes to durable storage.
    ///
    /// For in-memory stores, this is a no-op.
    /// For file-backed stores, this ensures data is persisted.
    fn flush(&self) -> PageStoreResult<()> {
        Ok(())
    }

    /// Save the commit's logical state to durable storage — the meta-slot flip.
    ///
    /// The page store owns `next_page_id` and the free-list (persisted as a page
    /// chain on this call), so the caller supplies only the **logical** root,
    /// version, and `next_table_id` — and `superseded`, the pages the tree has
    /// superseded and not yet freed, each batch with its `freed_version` (the last
    /// version holding it; see `CowBtree::save_state`), in the order the tree published
    /// them, so the versions rise. No root of `version` reaches
    /// them, so after a reopen they are free, and a store that persists its free
    /// list names them durably so that a close or a kill does not leak them. For
    /// in-memory stores this is a no-op; for the double-meta store it persists the
    /// free-list chain and the superseded pages, then writes the meta slot and fsyncs
    /// (the durable commit point).
    fn save_state(
        &self,
        _root: PageId,
        _version: u64,
        _next_table_id: u32,
        _superseded: &[(Vec<PageId>, u64)],
    ) -> PageStoreResult<()> {
        Ok(())
    }

    /// Fork this store into an independent in-memory [`CowPageStore`] holding the
    /// same pages, or `None` if the backend can't be forked in memory (a
    /// file/OPFS-backed store returns `None`).
    ///
    /// Pages carry `Arc`-shared entry vectors, so the fork bumps refcounts rather
    /// than deep-copying row values; a later write on *either* store copies only
    /// the touched page (`Arc::make_mut` sees the shared refcount and clones),
    /// leaving the other store untouched — the same immutability guarantee MVCC
    /// snapshots rely on, applied across two stores. The two stores keep
    /// independent page-id spaces (each is its own map), so their allocators never
    /// collide. This is the per-test-fixture fork primitive: seed a base store
    /// once, then fork a cheap isolated copy per test.
    ///
    /// [`CowPageStore`]: super::cow::CowPageStore
    fn fork_in_memory(&self) -> Option<Box<dyn PageStore>> {
        None
    }
}

/// Metadata for a page store, used for persistence — the **fixed-size** state held
/// in the meta slot.
///
/// Since PGM5 the variable-size catalog, table mappings, and row-id counters live
/// in the B+tree as `0x02` system entries (atomic with data, MVCC for free), and
/// the free-list lives in a chained page list whose head and that head page's checksum
/// (`freelist_head`, `freelist_crc`) are the only free-list state in the slot. The slot also carries the retired-page log's
/// newest entries after this record, a count the double-meta store caps, so the slot
/// is bounded — the old ~64 KiB ceiling is gone, structurally rather than by a
/// runtime cap.
#[derive(Debug, Clone, Default)]
pub struct PageStoreMetadata {
    /// Next page ID to allocate.
    pub next_page_id: PageId,
    /// Current root page ID.
    pub root: PageId,
    /// Current version number.
    pub version: u64,
    /// Next table ID to allocate.
    pub next_table_id: u32,
    /// Head of the free-list page chain (`NULL_PAGE_ID` when the free-list is
    /// empty). The chain is walked on open to rebuild the in-memory free-list.
    pub freelist_head: PageId,
    /// The checksum the head chain page carries (0 when there is none). Each chain
    /// page carries its successor's the same way, so the slot vouches for the whole
    /// chain: a page left from another commit fails the check the slot's word sets.
    pub freelist_crc: u32,
    /// Head of the **retired-page log**'s spilled chain (`NULL_PAGE_ID` when none):
    /// pages superseded and not yet freed when the slot was written, each with the
    /// version that last held it. The newest entries ride in the slot itself; the
    /// rest were spilled into this chain, which later saves extend and never rewrite.
    pub retired_head: PageId,
    /// The checksum the retired chain's head page carries (0 when there is none).
    pub retired_crc: u32,
    /// The largest `freed_version` the retired chain's head page lists — the walk
    /// reads the page only if it is at or past `retired_floor`.
    pub retired_max: u64,
    /// The smallest `freed_version` still retired when the slot was written: a
    /// logged entry below it was freed since (it is on the free list, or live again),
    /// so a reopen takes only the entries at or past it.
    pub retired_floor: u64,
}

/// A bounds-checked little-endian read cursor over a serialized [`PageStoreMetadata`]
/// blob. Every read advances the offset and returns `None` past the end, so the
/// deserializer is pure `?`-chaining with no manual offset arithmetic — this is the
/// **on-disk format**, so the read shape stays byte-for-byte the hand-rolled one.
struct MetaReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> MetaReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    /// Read the next `n` bytes, advancing the cursor; `None` if fewer remain.
    fn read_bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(n)?;
        let slice = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(slice)
    }

    fn read_u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.read_bytes(4)?.try_into().ok()?))
    }

    fn read_u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.read_bytes(8)?.try_into().ok()?))
    }
}

impl PageStoreMetadata {
    /// The serialized length, fixed.
    pub const SERIALIZED_LEN: usize = 72;

    /// Serialize metadata to bytes — the fixed 72-byte head of a slot payload.
    ///
    /// Format (since `PGMS`, which added the four `retired_*` fields; `PGMR` added
    /// `freelist_crc`):
    /// - Magic: "PGBT" (4 bytes)
    /// - `next_page_id`: u64 (8 bytes)
    /// - root: u64 (8 bytes)
    /// - version: u64 (8 bytes)
    /// - `next_table_id`: u32 (4 bytes)
    /// - `freelist_head`: u64 (8 bytes)
    /// - `freelist_crc`: u32 (4 bytes)
    /// - `retired_head`: u64 (8 bytes)
    /// - `retired_crc`: u32 (4 bytes)
    /// - `retired_max`: u64 (8 bytes)
    /// - `retired_floor`: u64 (8 bytes)
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::SERIALIZED_LEN);

        // Magic number: "PGBT" (PGlet B-Tree)
        buf.extend_from_slice(b"PGBT");

        buf.extend_from_slice(&self.next_page_id.to_le_bytes());
        buf.extend_from_slice(&self.root.to_le_bytes());
        buf.extend_from_slice(&self.version.to_le_bytes());
        buf.extend_from_slice(&self.next_table_id.to_le_bytes());
        buf.extend_from_slice(&self.freelist_head.to_le_bytes());
        buf.extend_from_slice(&self.freelist_crc.to_le_bytes());
        buf.extend_from_slice(&self.retired_head.to_le_bytes());
        buf.extend_from_slice(&self.retired_crc.to_le_bytes());
        buf.extend_from_slice(&self.retired_max.to_le_bytes());
        buf.extend_from_slice(&self.retired_floor.to_le_bytes());

        buf
    }

    /// Deserialize metadata from bytes. Returns `None` on any truncation or a bad
    /// magic — the read cursor bounds-checks every field.
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        let mut r = MetaReader::new(bytes);

        // Magic: "PGBT". A short or mismatched header fails loud here.
        if r.read_bytes(4)? != b"PGBT" {
            return None;
        }

        let next_page_id = r.read_u64()?;
        let root = r.read_u64()?;
        let version = r.read_u64()?;
        let next_table_id = r.read_u32()?;
        let freelist_head = r.read_u64()?;
        let freelist_crc = r.read_u32()?;
        let retired_head = r.read_u64()?;
        let retired_crc = r.read_u32()?;
        let retired_max = r.read_u64()?;
        let retired_floor = r.read_u64()?;

        Some(Self {
            next_page_id,
            root,
            version,
            next_table_id,
            freelist_head,
            freelist_crc,
            retired_head,
            retired_crc,
            retired_max,
            retired_floor,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_serialize_deserialize() {
        let meta = PageStoreMetadata {
            next_page_id: 42,
            root: 5,
            version: 100,
            next_table_id: 3,
            freelist_head: 7,
            freelist_crc: 0xDEAD_BEEF,
            retired_head: 9,
            retired_crc: 0xFEED_F00D,
            retired_max: 90,
            retired_floor: 80,
        };

        let bytes = meta.serialize();
        assert_eq!(
            bytes.len(),
            PageStoreMetadata::SERIALIZED_LEN,
            "the metadata is fixed-size"
        );
        let restored = PageStoreMetadata::deserialize(&bytes).unwrap();

        assert_eq!(restored.next_page_id, 42);
        assert_eq!(restored.root, 5);
        assert_eq!(restored.version, 100);
        assert_eq!(restored.next_table_id, 3);
        assert_eq!(restored.freelist_head, 7);
        assert_eq!(restored.freelist_crc, 0xDEAD_BEEF);
        assert_eq!(restored.retired_head, 9);
        assert_eq!(restored.retired_crc, 0xFEED_F00D);
        assert_eq!(restored.retired_max, 90);
        assert_eq!(restored.retired_floor, 80);
    }

    #[test]
    fn test_metadata_empty() {
        let meta = PageStoreMetadata {
            next_page_id: 1,
            root: u64::MAX,
            version: 0,
            next_table_id: 1,
            freelist_head: u64::MAX,
            freelist_crc: 0,
            ..PageStoreMetadata::default()
        };

        let bytes = meta.serialize();
        let restored = PageStoreMetadata::deserialize(&bytes).unwrap();

        assert_eq!(restored.next_page_id, 1);
        assert_eq!(restored.next_table_id, 1);
        assert_eq!(restored.freelist_head, u64::MAX);
    }

    #[test]
    fn test_metadata_invalid_magic() {
        let bytes = b"XXXX12345678901234567890123456";
        assert!(PageStoreMetadata::deserialize(bytes).is_none());
    }

    #[test]
    fn test_metadata_too_short() {
        let bytes = b"PGBT";
        assert!(PageStoreMetadata::deserialize(bytes).is_none());
    }
}
