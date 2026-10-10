//! B+tree page types and serialization.
//!
//! Pages are the fundamental unit of storage. Each page is a fixed size
//! (default 4KB) containing either:
//! - Internal node: keys and child page pointers
//! - Leaf node: keys and row data — each value inline, or a reference to an
//!   overflow chain
//! - Overflow: one link of a chain holding a leaf value too large to keep inline
//! - Free: reclaimed page available for reuse
//!
//! # Page Layout
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │ PageHeader (32 bytes)                                       │
//! │ ┌─────────┬─────────┬─────────┬─────────┬─────────────────┐ │
//! │ │ page_id │  flags  │  count  │ reserved│    checksum     │ │
//! │ │  (u64)  │  (u16)  │  (u16)  │  (u64)  │     (u32)       │ │
//! │ └─────────┴─────────┴─────────┴─────────┴─────────────────┘ │
//! ├─────────────────────────────────────────────────────────────┤
//! │ Data (PAGE_SIZE - 32 bytes)                                 │
//! │                                                             │
//! │ Internal: [first_child] [klen0,key0,child0] [klen1,key1,...] │
//! │ Leaf:     [klen0,key0, vlen0,val0] [klen1,key1, vlen1,val1]  │
//! │ Overflow: [payload bytes; count]                            │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! A leaf value is its postcard encoding, `[vlen: u32][bytes]`, when that encoding
//! is at most [`INLINE_VALUE_MAX`] bytes and the entry fits a page with it; any other
//! value is stored **out of line**, in a chain of overflow pages each carrying up to
//! [`DATA_SIZE`] bytes of the encoding (`count` in the header, the next link in
//! `next_or_reserved`), and the leaf holds a reference to it: `vlen` is
//! [`OVERFLOW_TAG`], followed by the chain's head page (`u64`) and the encoding's
//! total length (`u64`). A chain is written with the batch that creates it and freed
//! like any other page `CowBtree` supersedes, when the entry is deleted or replaced.
//!
//! Keys are **variable-length** ([`BtreeKey`] holds data rows and index
//! entries), so both node kinds length-prefix each key with a `u32` byte
//! count. Whether a node splits is byte-aware (see [`leaf_entries_fit`] /
//! [`internal_entries_fit`], which measure the serialized entries against
//! [`DATA_SIZE`]); a count cap ([`MAX_ENTRIES_PER_PAGE`]) is a secondary bound only.
//! Where it splits is byte-aware too: [`split_points`] cuts the entries into
//! pieces that each fit, so every page `CowBtree` builds fits one page as long as
//! every entry fits alone — which `CowBtree::apply_batch` checks before it
//! touches the tree ([`MAX_KEY_LEN`] for a key, which must also fit an internal
//! page as a separator). [`Page::serialize`] is fallible regardless: a page that
//! does not fit is a [`PageStoreError::Overfull`], never a write past the buffer.

use std::sync::Arc;

use super::BtreeKey;
use super::store::{PageStoreError, PageStoreResult};
use pglet_sql::types::Value;

/// Default page size in bytes (4KB).
pub const PAGE_SIZE: usize = 4096;

/// Page header size in bytes.
pub const HEADER_SIZE: usize = 32;

/// Usable data area per page.
pub const DATA_SIZE: usize = PAGE_SIZE - HEADER_SIZE;

/// Byte size of the `u32` length prefix that precedes each key on a page.
pub const KEY_LEN_PREFIX: usize = 4;

/// Byte size of a child pointer (`PageId`) on an internal page.
const CHILD_SIZE: usize = 8;

/// The largest encoded key a page can hold in **every** position it may take.
///
/// A key in a leaf may be promoted as the separator of an internal page, and an
/// internal page must hold at least one separator beside its `first_child`:
/// `first_child` (8) + `[key_len]` (4) + the key + its child pointer (8) must fit
/// [`DATA_SIZE`], so a key is at most `4064 - 20 = 4044` bytes. A longer key cannot
/// be stored without key overflow or separator truncation, neither of which pglet
/// has, so `CowBtree::apply_batch` refuses it before it touches the tree.
pub const MAX_KEY_LEN: usize = DATA_SIZE - CHILD_SIZE - KEY_LEN_PREFIX - CHILD_SIZE;

/// Byte size of the `u32` length prefix that precedes each leaf value.
const VALUE_LEN_PREFIX: usize = 4;

/// The value-length word that marks a leaf value stored out of line. No inline value
/// comes near it: an inline value is shorter than a page.
pub const OVERFLOW_TAG: u32 = u32::MAX;

/// Byte size of an out-of-line reference after its [`OVERFLOW_TAG`]: the chain's head
/// page (`u64`) and the value encoding's total length (`u64`).
const OVERFLOW_REF_SIZE: usize = 16;

/// The longest leaf value encoding kept inline; a longer one goes to an overflow
/// chain.
///
/// A quarter page, the analogue of 18.6's `TOAST_TUPLE_THRESHOLD`: small enough that a
/// leaf of inline entries still holds several, and — what correctness needs of it —
/// below half a page, so no inline data row is ever the entry a two-way cut cannot
/// place. Where the line sits is performance, not what a query answers.
pub const INLINE_VALUE_MAX: usize = DATA_SIZE / 4;

/// Serialized size of a length-prefixed key on a page.
#[inline]
fn key_slot_size(key: &BtreeKey) -> usize {
    KEY_LEN_PREFIX + key.encoded_len()
}

/// A bounds-checked cursor over a page buffer: every write that would pass the
/// end of the buffer is refused rather than indexing past it.
struct PageWriter {
    buf: Vec<u8>,
    offset: usize,
}

impl PageWriter {
    fn new() -> Self {
        Self {
            buf: vec![0u8; PAGE_SIZE],
            offset: HEADER_SIZE,
        }
    }

    /// Append `bytes`, or `None` if they would not fit the page.
    fn put(&mut self, bytes: &[u8]) -> Option<()> {
        let end = self.offset.checked_add(bytes.len())?;
        self.buf.get_mut(self.offset..end)?.copy_from_slice(bytes);
        self.offset = end;
        Some(())
    }

    /// Append a length-prefixed key: `[key_len: u32 LE][key bytes]`.
    fn put_key(&mut self, key: &BtreeKey) -> Option<()> {
        let key_len = u32::try_from(key.encoded_len()).ok()?;
        self.put(&key_len.to_le_bytes())?;
        let mut scratch = Vec::with_capacity(key.encoded_len());
        key.encode_into(&mut scratch);
        self.put(&scratch)
    }
}

/// A bounds-checked cursor over a page read back from the store: every read past
/// the end of the buffer is `None`, so a malformed page is refused rather than
/// indexed past.
struct PageReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> PageReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: HEADER_SIZE,
        }
    }

    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let end = self.offset.checked_add(len)?;
        let out = self.bytes.get(self.offset..end)?;
        self.offset = end;
        Some(out)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    /// Read a length-prefixed key, refusing one whose encoding does not consume
    /// exactly its length prefix.
    fn key(&mut self) -> Option<BtreeKey> {
        let key_len = self.u32()? as usize;
        let bytes = self.take(key_len)?;
        let (key, consumed) = BtreeKey::try_decode(bytes)?;
        (consumed == key_len).then_some(key)
    }
}

/// Unique identifier for a page within the storage file.
pub type PageId = u64;

/// Special page ID indicating null/no page.
pub const NULL_PAGE_ID: PageId = u64::MAX;

/// Page flags indicating page type and state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum PageFlags {
    /// Free page available for allocation.
    Free = 0,
    /// Internal B+tree node (keys + child pointers).
    Internal = 1,
    /// Leaf B+tree node (keys + values).
    Leaf = 2,
    /// One link of an overflow chain: part of the encoding of a leaf value too long
    /// to keep inline.
    Overflow = 3,
}

impl PageFlags {
    // arms are an explicit enumeration of the four valid discriminants; `0` is listed
    // for completeness and `_` is a documented "unknown -> Free" fallback, so the shared
    // body with `Free` is intentional and kept separate.
    #[allow(clippy::match_same_arms)]
    pub const fn from_u16(v: u16) -> Self {
        match v {
            0 => Self::Free,
            1 => Self::Internal,
            2 => Self::Leaf,
            3 => Self::Overflow,
            _ => Self::Free, // Default to free for unknown
        }
    }
}

/// Page header stored at the beginning of each page.
///
/// Total size: 32 bytes
/// - `page_id`: 8 bytes
/// - flags: 2 bytes
/// - count: 2 bytes (number of keys for internal/leaf; the data length for an overflow link)
/// - `next_or_reserved`: 8 bytes (the next leaf for a leaf, the next free page for a free
///   page, the next link for an overflow link; unused for internal)
/// - padding: 8 bytes
/// - checksum: 4 bytes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageHeader {
    /// This page's ID.
    pub page_id: PageId,
    /// Page type flags.
    pub flags: PageFlags,
    /// Number of entries (keys for internal/leaf); for an overflow link, its data length.
    pub count: u16,
    /// For leaf pages: next leaf page ID for range scans.
    /// For free pages: next free page ID in free list.
    /// For overflow links: the next link in the chain.
    /// For internal pages: unused.
    pub next_or_reserved: PageId,
    /// CRC32 checksum of page data.
    pub checksum: u32,
}

impl PageHeader {
    pub const SIZE: usize = HEADER_SIZE;

    /// Create a new page header.
    pub const fn new(page_id: PageId, flags: PageFlags) -> Self {
        Self {
            page_id,
            flags,
            count: 0,
            next_or_reserved: NULL_PAGE_ID,
            checksum: 0,
        }
    }

    /// Encode header to bytes.
    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..8].copy_from_slice(&self.page_id.to_le_bytes());
        buf[8..10].copy_from_slice(&(self.flags as u16).to_le_bytes());
        buf[10..12].copy_from_slice(&self.count.to_le_bytes());
        buf[12..20].copy_from_slice(&self.next_or_reserved.to_le_bytes());
        // bytes 20..28 reserved/padding
        buf[28..32].copy_from_slice(&self.checksum.to_le_bytes());
        buf
    }

    /// Decode header from bytes.
    // Fixed-layout decode: every `try_into` targets a statically-sized slice of
    // the 32-byte header, so none can fail.
    #[allow(clippy::unwrap_used)]
    pub fn decode(bytes: &[u8]) -> Self {
        let page_id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let flags = PageFlags::from_u16(u16::from_le_bytes(bytes[8..10].try_into().unwrap()));
        let count = u16::from_le_bytes(bytes[10..12].try_into().unwrap());
        let next_or_reserved = u64::from_le_bytes(bytes[12..20].try_into().unwrap());
        let checksum = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        Self {
            page_id,
            flags,
            count,
            next_or_reserved,
            checksum,
        }
    }
}

/// Entry in an internal node: a child pointer.
/// Internal nodes have N keys and N+1 children.
/// `children[i]` contains keys `< keys[i]`, `children[i+1]` contains keys `>= keys[i]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternalEntry {
    pub key: BtreeKey,
    pub child: PageId,
}

/// Entry in a leaf node: key + value.
#[derive(Clone, Debug, PartialEq)]
pub struct LeafEntry {
    pub key: BtreeKey,
    pub value: LeafValue,
}

/// A leaf entry's value as its page holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum LeafValue {
    /// The value itself, postcard-encoded on the page.
    Inline(Vec<Value>),
    /// A reference to the overflow chain holding the value's postcard encoding.
    Overflow(OverflowRef),
}

/// Where an out-of-line leaf value lives: the first page of its overflow chain and
/// the total length of the postcard encoding the chain carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverflowRef {
    /// The chain's first page.
    pub head: PageId,
    /// The value encoding's length in bytes, across the whole chain.
    pub len: u64,
}

impl OverflowRef {
    /// The number of pages a chain of this length has: every link but the last
    /// carries a full [`DATA_SIZE`] bytes.
    #[must_use]
    pub const fn page_count(self) -> u64 {
        self.len.div_ceil(DATA_SIZE as u64)
    }
}

/// A B+tree page.
#[derive(Clone, Debug)]
pub enum Page {
    /// Free page in the free list.
    Free { header: PageHeader },

    /// Internal node with keys and child pointers.
    /// Has `count` keys and `count + 1` children.
    /// `children[0]` holds keys `< keys[0]`;
    /// `children[i]` holds keys `>= keys[i-1]` and `< keys[i]`;
    /// `children[count]` holds keys `>= keys[count-1]`.
    Internal {
        header: PageHeader,
        /// First child (keys less than `keys[0]`).
        first_child: PageId,
        /// Key-child pairs. Entry `i` has `key[i]` and the child for keys `>= key[i]`.
        ///
        /// `Arc`-wrapped so descent (which only navigates internal pages, never
        /// mutates them) clones a page in O(1); copy-on-write pays the deep copy
        /// only on the leaf it actually mutates, via `Arc::make_mut`.
        entries: Arc<Vec<InternalEntry>>,
    },

    /// Leaf node with key-value pairs.
    Leaf {
        header: PageHeader,
        /// Key-value entries sorted by key.
        ///
        /// `Arc`-wrapped so a descent-time page clone is a refcount bump; the
        /// CoW deep copy happens only on the mutated leaf via `Arc::make_mut`.
        entries: Arc<Vec<LeafEntry>>,
    },

    /// One link of an overflow chain: up to [`DATA_SIZE`] bytes of a leaf value's
    /// encoding. `header.next_or_reserved` is the next link, [`NULL_PAGE_ID`] at the
    /// last; the header's `count` is `data`'s length on disk.
    Overflow {
        header: PageHeader,
        /// This link's share of the encoding. `Arc`-wrapped so a cache or fork clone
        /// is a refcount bump.
        data: Arc<Vec<u8>>,
    },
}

impl Page {
    /// Create a new free page.
    pub const fn new_free(page_id: PageId, next_free: PageId) -> Self {
        let mut header = PageHeader::new(page_id, PageFlags::Free);
        header.next_or_reserved = next_free;
        Self::Free { header }
    }

    /// Create a new empty internal page.
    // Not `const`: `Arc::new` is not a const fn.
    pub fn new_internal(page_id: PageId) -> Self {
        Self::Internal {
            header: PageHeader::new(page_id, PageFlags::Internal),
            first_child: NULL_PAGE_ID,
            entries: Arc::new(Vec::new()),
        }
    }

    /// Create a new empty leaf page.
    // Not `const`: `Arc::new` is not a const fn.
    pub fn new_leaf(page_id: PageId) -> Self {
        let mut header = PageHeader::new(page_id, PageFlags::Leaf);
        header.next_or_reserved = NULL_PAGE_ID; // next_leaf
        Self::Leaf {
            header,
            entries: Arc::new(Vec::new()),
        }
    }

    /// Create one overflow-chain link carrying `data`, linked to `next`.
    pub fn new_overflow(page_id: PageId, data: Vec<u8>, next: PageId) -> Self {
        let mut header = PageHeader::new(page_id, PageFlags::Overflow);
        header.count = u16::try_from(data.len()).unwrap_or(u16::MAX);
        header.next_or_reserved = next;
        Self::Overflow {
            header,
            data: Arc::new(data),
        }
    }

    /// Get the page ID.
    pub const fn page_id(&self) -> PageId {
        match self {
            Self::Free { header }
            | Self::Internal { header, .. }
            | Self::Leaf { header, .. }
            | Self::Overflow { header, .. } => header.page_id,
        }
    }

    /// Get the page header.
    pub const fn header(&self) -> &PageHeader {
        match self {
            Self::Free { header }
            | Self::Internal { header, .. }
            | Self::Leaf { header, .. }
            | Self::Overflow { header, .. } => header,
        }
    }

    /// Get mutable header.
    pub const fn header_mut(&mut self) -> &mut PageHeader {
        match self {
            Self::Free { header }
            | Self::Internal { header, .. }
            | Self::Leaf { header, .. }
            | Self::Overflow { header, .. } => header,
        }
    }

    /// Check if this is a leaf page.
    pub const fn is_leaf(&self) -> bool {
        matches!(self, Self::Leaf { .. })
    }

    /// Check if this is an internal page.
    pub const fn is_internal(&self) -> bool {
        matches!(self, Self::Internal { .. })
    }

    /// Get number of keys in this page.
    pub fn key_count(&self) -> usize {
        match self {
            Self::Free { .. } | Self::Overflow { .. } => 0,
            Self::Internal { entries, .. } => entries.len(),
            Self::Leaf { entries, .. } => entries.len(),
        }
    }

    /// The number of bytes this page serializes to, header included — the quantity
    /// [`Page::serialize`] refuses above [`PAGE_SIZE`].
    ///
    /// Computed from the same per-entry measures the split decisions use
    /// ([`leaf_entries_fit`] / [`internal_entries_fit`]), without allocating.
    pub fn encoded_len(&self) -> usize {
        match self {
            Self::Free { .. } => HEADER_SIZE,
            Self::Internal { entries, .. } => {
                HEADER_SIZE
                    + CHILD_SIZE
                    + entries
                        .iter()
                        .map(|e| internal_entry_bytes(&e.key))
                        .sum::<usize>()
            }
            Self::Leaf { entries, .. } => {
                HEADER_SIZE
                    + entries
                        .iter()
                        .map(|e| leaf_entry_bytes(&e.key, &e.value))
                        .sum::<usize>()
            }
            Self::Overflow { data, .. } => HEADER_SIZE + data.len(),
        }
    }

    /// Refuse a page whose serialized form would not fit one page.
    ///
    /// # Errors
    ///
    /// [`PageStoreError::Overfull`] naming the page and its size.
    pub fn check_fits(&self) -> PageStoreResult<()> {
        let bytes = self.encoded_len();
        if bytes > PAGE_SIZE {
            return Err(PageStoreError::Overfull {
                page_id: self.page_id(),
                bytes,
            });
        }
        Ok(())
    }

    /// Serialize page to bytes.
    ///
    /// Every write is bounds-checked, so a page whose entries do not fit is refused
    /// rather than written past the buffer.
    ///
    /// # Errors
    ///
    /// [`PageStoreError::Overfull`] if the page does not fit [`PAGE_SIZE`], or
    /// [`PageStoreError::Io`] if a leaf value fails to serialize.
    pub fn serialize(&self) -> PageStoreResult<Vec<u8>> {
        let overfull = || PageStoreError::Overfull {
            page_id: self.page_id(),
            bytes: self.encoded_len(),
        };
        let mut w = PageWriter::new();

        let header = match self {
            Self::Free { header } => *header,

            Self::Internal {
                header,
                first_child,
                entries,
            } => {
                w.put(&first_child.to_le_bytes()).ok_or_else(overfull)?;
                // Entries: [key_len (4 bytes), key, child (8 bytes)] tuples.
                for entry in entries.iter() {
                    w.put_key(&entry.key).ok_or_else(overfull)?;
                    w.put(&entry.child.to_le_bytes()).ok_or_else(overfull)?;
                }
                let mut h = *header;
                h.count = u16::try_from(entries.len()).map_err(|_| overfull())?;
                h
            }

            Self::Leaf { header, entries } => {
                // Entries: [key_len (4), key, value_len (4 bytes), value_data].
                for entry in entries.iter() {
                    w.put_key(&entry.key).ok_or_else(overfull)?;
                    match &entry.value {
                        LeafValue::Inline(value) => {
                            let value_bytes = encode_value(value)?;
                            let value_len = u32::try_from(value_bytes.len())
                                .ok()
                                .filter(|&len| len != OVERFLOW_TAG)
                                .ok_or_else(overfull)?;
                            w.put(&value_len.to_le_bytes()).ok_or_else(overfull)?;
                            w.put(&value_bytes).ok_or_else(overfull)?;
                        }
                        LeafValue::Overflow(r) => {
                            w.put(&OVERFLOW_TAG.to_le_bytes()).ok_or_else(overfull)?;
                            w.put(&r.head.to_le_bytes()).ok_or_else(overfull)?;
                            w.put(&r.len.to_le_bytes()).ok_or_else(overfull)?;
                        }
                    }
                }
                let mut h = *header;
                h.count = u16::try_from(entries.len()).map_err(|_| overfull())?;
                h
            }

            Self::Overflow { header, data } => {
                w.put(data).ok_or_else(overfull)?;
                let mut h = *header;
                h.count = u16::try_from(data.len()).map_err(|_| overfull())?;
                h
            }
        };

        let mut buf = w.buf;
        buf[..HEADER_SIZE].copy_from_slice(&header.encode());
        Ok(buf)
    }

    /// Deserialize page from bytes, or `None` if they do not decode as a page.
    ///
    /// Every read is bounds-checked and every value is decoded fallibly, so a page
    /// whose bytes are not a page this codec wrote is refused, never indexed past
    /// or `expect`ed through. `DoubleMetaStore` verifies a page's checksum before
    /// decoding it and maps a `None` here to [`PageStoreError::Corruption`].
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        let header = PageHeader::decode(bytes.get(..HEADER_SIZE)?);
        let mut r = PageReader::new(bytes);

        match header.flags {
            PageFlags::Free => Some(Self::Free { header }),

            PageFlags::Internal => {
                let first_child = r.u64()?;
                let mut entries = Vec::with_capacity(usize::from(header.count));
                for _ in 0..header.count {
                    let key = r.key()?;
                    let child = r.u64()?;
                    entries.push(InternalEntry { key, child });
                }
                Some(Self::Internal {
                    header,
                    first_child,
                    entries: Arc::new(entries),
                })
            }

            PageFlags::Leaf => {
                let mut entries = Vec::with_capacity(usize::from(header.count));
                for _ in 0..header.count {
                    let key = r.key()?;
                    let value_len = r.u32()?;
                    let value = if value_len == OVERFLOW_TAG {
                        let head = r.u64()?;
                        let len = r.u64()?;
                        // An empty chain is written by nothing: a value's encoding is
                        // never empty, and one short enough to fit stays inline.
                        (len > 0 && head != NULL_PAGE_ID)
                            .then_some(LeafValue::Overflow(OverflowRef { head, len }))?
                    } else {
                        LeafValue::Inline(postcard::from_bytes(r.take(value_len as usize)?).ok()?)
                    };
                    entries.push(LeafEntry { key, value });
                }
                Some(Self::Leaf {
                    header,
                    entries: Arc::new(entries),
                })
            }

            PageFlags::Overflow => {
                let count = usize::from(header.count);
                if count == 0 || count > DATA_SIZE {
                    return None;
                }
                let data = r.take(count)?.to_vec();
                Some(Self::Overflow {
                    header,
                    data: Arc::new(data),
                })
            }
        }
    }
}

/// Hard count cap on entries per page.
///
/// Variable-length keys make byte budget the real limit (see
/// [`leaf_entries_fit`]/[`internal_entries_fit`]); this cap is a cheap
/// secondary bound that keeps a page of tiny entries from growing an
/// unbounded `Vec` and keeps `count: u16` in range.
pub const MAX_ENTRIES_PER_PAGE: usize = 2048;

/// Serialized byte size of one internal entry: `[key_len][key][child]`.
#[inline]
pub(super) fn internal_entry_bytes(key: &BtreeKey) -> usize {
    key_slot_size(key) + CHILD_SIZE
}

/// A leaf value's postcard encoding — what an inline value writes on its page and an
/// overflow chain carries.
///
/// # Errors
///
/// [`PageStoreError::Io`] if the value fails to encode.
pub fn encode_value(value: &[Value]) -> PageStoreResult<Vec<u8>> {
    postcard::to_allocvec(value)
        .map_err(|e| PageStoreError::Io(format!("leaf value serialization failed: {e}")))
}

/// The length of a leaf value's postcard encoding, counted without allocating.
///
/// This runs per entry on every `leaf_entries_fit` split check (i.e. per insert), so
/// the allocation `to_allocvec` would do dominated the batched-write apply loop.
pub(super) fn encoded_value_len(value: &[Value]) -> usize {
    postcard::experimental::serialized_size(value).unwrap_or(0)
}

/// Serialized byte size of one leaf entry: `[key_len][key][value_len][value]`, the
/// value being its inline encoding or an out-of-line reference.
///
/// An inline value is measured with the same postcard encoding `serialize` uses, so
/// the estimate is exact.
pub(super) fn leaf_entry_bytes(key: &BtreeKey, value: &LeafValue) -> usize {
    key_slot_size(key)
        + VALUE_LEN_PREFIX
        + match value {
            LeafValue::Inline(v) => encoded_value_len(v),
            LeafValue::Overflow(_) => OVERFLOW_REF_SIZE,
        }
}

/// Serialized byte size of a leaf entry holding `key` and an out-of-line reference.
#[must_use]
pub(super) const fn leaf_entry_bytes_out_of_line(key_len: usize) -> usize {
    KEY_LEN_PREFIX + key_len + VALUE_LEN_PREFIX + OVERFLOW_REF_SIZE
}

/// Serialized byte size of a leaf entry holding `key` and a value whose encoding is
/// `value_len` bytes, inline.
#[must_use]
pub(super) const fn leaf_entry_bytes_inline(key_len: usize, value_len: usize) -> usize {
    KEY_LEN_PREFIX + key_len + VALUE_LEN_PREFIX + value_len
}

/// Whether `entries` (internal node) serialize within the page data budget.
///
/// Internal pages also store `first_child` (8 bytes) ahead of the entries.
/// Returns `false` once the serialized page would exceed [`DATA_SIZE`] or the
/// [`MAX_ENTRIES_PER_PAGE`] count cap — the caller should split.
#[must_use]
pub fn internal_entries_fit(entries: &[InternalEntry]) -> bool {
    if entries.len() > MAX_ENTRIES_PER_PAGE {
        return false;
    }
    let mut used = CHILD_SIZE; // first_child
    for e in entries {
        used += internal_entry_bytes(&e.key);
        if used > DATA_SIZE {
            return false;
        }
    }
    used <= DATA_SIZE
}

/// Whether `entries` (leaf node) serialize within the page data budget.
///
/// Returns `false` once the serialized page would exceed [`DATA_SIZE`] or the
/// [`MAX_ENTRIES_PER_PAGE`] count cap — the caller should split.
#[must_use]
pub fn leaf_entries_fit(entries: &[LeafEntry]) -> bool {
    if entries.len() > MAX_ENTRIES_PER_PAGE {
        return false;
    }
    let mut used = 0usize;
    for e in entries {
        used += leaf_entry_bytes(&e.key, &e.value);
        if used > DATA_SIZE {
            return false;
        }
    }
    used <= DATA_SIZE
}

/// Whether a piece of `count` entries totalling `bytes` (its fixed part included)
/// fits one page.
const fn piece_fits(bytes: usize, count: usize) -> bool {
    bytes <= DATA_SIZE && count <= MAX_ENTRIES_PER_PAGE
}

/// Where to cut a node whose entries no longer fit one page, so that every piece
/// does.
///
/// `sizes` are the entries' serialized sizes in key order; `base` is the fixed part
/// every piece carries (an internal page's `first_child`, `0` for a leaf). The
/// result is the ascending cut indices: a leaf's pieces are `[0, c1)`, `[c1, c2)`,
/// …, `[ck, n)`. With `promote` (an internal node) the entry **at** each cut moves
/// up to the parent as the separator and belongs to no piece — the pieces are
/// `[0, c1)`, `[c1 + 1, c2)`, … — so either side of a cut may be empty, a page
/// holding only its `first_child`.
///
/// The best byte-balanced two-way cut is taken when both sides fit. It is not
/// always enough: a page of at most `D` bytes that gains one entry of `E` bytes has
/// a larger half of up to `D/2 + E` bytes after the most balanced cut, which
/// exceeds `D` once `E > D/2` — an index key 18.6's item limit admits reaches
/// about 2 700 bytes against a `D/2` of 2 032. The fallback is a greedy left-to-right
/// fill, which fits every piece whenever every entry fits alone (the caller's
/// precondition: `CowBtree::apply_batch` refuses an entry that does not). An entry
/// that does not fit alone still gets a piece of its own, which the page store then
/// refuses as [`PageStoreError::Overfull`] rather than writing it.
#[must_use]
pub(super) fn split_points(sizes: &[usize], base: usize, promote: bool) -> Vec<usize> {
    let n = sizes.len();
    let total: usize = sizes.iter().sum();

    // Best balanced two-way cut. A leaf cut keeps both sides non-empty; an internal
    // cut may leave either side with no entries (the promoted entry is the cut).
    let range = if promote { 0..n } else { 1..n };
    let mut best: Option<(usize, usize)> = None;
    let mut left = 0usize;
    for (i, &size) in sizes.iter().enumerate() {
        if range.contains(&i) {
            let promoted = if promote { size } else { 0 };
            let right = total - left - promoted;
            let (left_count, right_count) = if promote { (i, n - i - 1) } else { (i, n - i) };
            if piece_fits(base + left, left_count) && piece_fits(base + right, right_count) {
                let worst = left.max(right);
                if best.is_none_or(|(w, _)| worst < w) {
                    best = Some((worst, i));
                }
            }
        }
        left += size;
    }
    if let Some((_, cut)) = best {
        return vec![cut];
    }

    // Greedy fill: close a piece as soon as the next entry would overflow it.
    let mut cuts = Vec::new();
    let mut bytes = base;
    let mut count = 0usize;
    let mut i = 0;
    while i < n {
        if count > 0 && !piece_fits(bytes + sizes[i], count + 1) {
            cuts.push(i);
            bytes = base;
            count = 0;
            if promote {
                // The entry at the cut is the separator; the next piece starts after it.
                i += 1;
            }
            continue;
        }
        bytes += sizes[i];
        count += 1;
        i += 1;
    }
    cuts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_page_header_encode_decode() {
        let header = PageHeader {
            page_id: 42,
            flags: PageFlags::Leaf,
            count: 10,
            next_or_reserved: 100,
            checksum: 0xDEAD_BEEF,
        };

        let encoded = header.encode();
        let decoded = PageHeader::decode(&encoded);

        assert_eq!(header, decoded);
    }

    #[test]
    fn test_free_page_serialize() {
        let page = Page::new_free(5, 10);
        let bytes = page.serialize().unwrap();
        let restored = Page::deserialize(&bytes).unwrap();

        assert!(matches!(restored, Page::Free { .. }));
        assert_eq!(restored.page_id(), 5);
        if let Page::Free { header } = restored {
            assert_eq!(header.next_or_reserved, 10);
        }
    }

    #[test]
    fn test_leaf_page_serialize() {
        let mut page = Page::new_leaf(1);
        if let Page::Leaf { entries, .. } = &mut page {
            let entries = Arc::make_mut(entries);
            entries.push(LeafEntry {
                key: BtreeKey::new(1, 100),
                value: LeafValue::Inline(vec![Value::Int4(42), Value::Text("hello".to_string())]),
            });
            entries.push(LeafEntry {
                key: BtreeKey::new(1, 200),
                value: LeafValue::Inline(vec![Value::Int4(99)]),
            });
        }

        let bytes = page.serialize().unwrap();
        let restored = Page::deserialize(&bytes).unwrap();

        assert!(restored.is_leaf());
        assert_eq!(restored.key_count(), 2);

        if let Page::Leaf { entries, .. } = restored {
            assert_eq!(entries[0].key, BtreeKey::new(1, 100));
            assert_eq!(
                entries[0].value,
                LeafValue::Inline(vec![Value::Int4(42), Value::Text("hello".to_string())])
            );
            assert_eq!(entries[1].key, BtreeKey::new(1, 200));
            assert_eq!(entries[1].value, LeafValue::Inline(vec![Value::Int4(99)]));
        }
    }

    #[test]
    fn test_internal_page_serialize() {
        let mut page = Page::new_internal(2);
        if let Page::Internal {
            first_child,
            entries,
            ..
        } = &mut page
        {
            *first_child = 10;
            let entries = Arc::make_mut(entries);
            entries.push(InternalEntry {
                key: BtreeKey::new(1, 50),
                child: 11,
            });
            entries.push(InternalEntry {
                key: BtreeKey::new(1, 100),
                child: 12,
            });
        }

        let bytes = page.serialize().unwrap();
        let restored = Page::deserialize(&bytes).unwrap();

        assert!(restored.is_internal());
        assert_eq!(restored.key_count(), 2);

        if let Page::Internal {
            first_child,
            entries,
            ..
        } = restored
        {
            assert_eq!(first_child, 10);
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].key, BtreeKey::new(1, 50));
            assert_eq!(entries[0].child, 11);
            assert_eq!(entries[1].key, BtreeKey::new(1, 100));
            assert_eq!(entries[1].child, 12);
        }
    }

    #[test]
    fn test_page_size_constants() {
        assert_eq!(PAGE_SIZE, 4096);
        assert_eq!(HEADER_SIZE, 32);
        assert_eq!(DATA_SIZE, 4064);
    }

    #[test]
    fn test_index_key_page_roundtrip() {
        // A leaf and an internal page holding index-entry keys must round-trip.
        let mut leaf = Page::new_leaf(7);
        if let Page::Leaf { entries, .. } = &mut leaf {
            let entries = Arc::make_mut(entries);
            entries.push(LeafEntry {
                key: BtreeKey::index(3, b"alice", 100),
                value: LeafValue::Inline(vec![]), // index entries store an empty value
            });
            entries.push(LeafEntry {
                key: BtreeKey::index(3, b"bob", 200),
                value: LeafValue::Inline(vec![]),
            });
        }
        let restored = Page::deserialize(&leaf.serialize().unwrap()).unwrap();
        if let Page::Leaf { entries, .. } = restored {
            assert_eq!(entries[0].key, BtreeKey::index(3, b"alice", 100));
            assert_eq!(entries[1].key, BtreeKey::index(3, b"bob", 200));
        } else {
            panic!("expected leaf");
        }

        let mut internal = Page::new_internal(8);
        if let Page::Internal {
            first_child,
            entries,
            ..
        } = &mut internal
        {
            *first_child = 1;
            Arc::make_mut(entries).push(InternalEntry {
                key: BtreeKey::index(3, b"midpoint-value", 42),
                child: 2,
            });
        }
        let restored = Page::deserialize(&internal.serialize().unwrap()).unwrap();
        if let Page::Internal { entries, .. } = restored {
            assert_eq!(entries[0].key, BtreeKey::index(3, b"midpoint-value", 42));
        } else {
            panic!("expected internal");
        }
    }

    #[test]
    fn test_large_index_key_page_fill_never_overflows() {
        // Insert entries with LARGE index keys (here, 200-byte values) and
        // verify the byte-aware fit check refuses to overflow a 4KB page and
        // that whatever fits round-trips exactly. This is the regression guard
        // for variable-length keys silently overflowing the page.
        let big_value = vec![0xABu8; 200];

        // Leaf: keep adding until leaf_entries_fit says stop, then assert the
        // page that fit serializes within PAGE_SIZE and round-trips.
        let mut entries: Vec<LeafEntry> = Vec::new();
        let mut last_fitting: Vec<LeafEntry> = Vec::new();
        for i in 0..1000u64 {
            entries.push(LeafEntry {
                key: BtreeKey::index(1, &big_value, i),
                value: LeafValue::Inline(vec![]),
            });
            if leaf_entries_fit(&entries) {
                last_fitting = entries.clone();
            } else {
                break;
            }
        }
        // With ~217-byte slots and a 4064-byte budget, ~18 large entries fit:
        // far below the old count cap of 30, which would have overflowed.
        assert!(
            !last_fitting.is_empty() && last_fitting.len() < 30,
            "large index keys must trip the byte budget well below the count cap, got {}",
            last_fitting.len()
        );

        let mut page = Page::new_leaf(99);
        if let Page::Leaf { entries: e, .. } = &mut page {
            *e = Arc::new(last_fitting.clone());
        }
        let bytes = page.serialize().unwrap();
        assert_eq!(bytes.len(), PAGE_SIZE, "serialized page must be one page");
        let restored = Page::deserialize(&bytes).unwrap();
        if let Page::Leaf { entries: e, .. } = restored {
            assert_eq!(e.len(), last_fitting.len());
            for (a, b) in e.iter().zip(last_fitting.iter()) {
                assert_eq!(a.key, b.key);
            }
        } else {
            panic!("expected leaf");
        }

        // Internal: same guard for [key_len][key][child] tuples.
        let mut ientries: Vec<InternalEntry> = Vec::new();
        let mut last_internal: Vec<InternalEntry> = Vec::new();
        for i in 0..1000u64 {
            ientries.push(InternalEntry {
                key: BtreeKey::index(1, &big_value, i),
                child: i + 1,
            });
            if internal_entries_fit(&ientries) {
                last_internal = ientries.clone();
            } else {
                break;
            }
        }
        let mut ipage = Page::new_internal(98);
        if let Page::Internal { entries: e, .. } = &mut ipage {
            *e = Arc::new(last_internal.clone());
        }
        let ibytes = ipage.serialize().unwrap();
        assert_eq!(ibytes.len(), PAGE_SIZE);
        let restored = Page::deserialize(&ibytes).unwrap();
        if let Page::Internal { entries: e, .. } = restored {
            assert_eq!(e.len(), last_internal.len());
        } else {
            panic!("expected internal");
        }
    }

    #[test]
    fn test_data_page_byte_budget_matches_old_count() {
        // A page full of small data-row keys (~21-byte leaf slots for an empty
        // value) must fit comfortably; the byte budget only bites on big keys.
        let mut entries: Vec<LeafEntry> = Vec::new();
        for i in 0..30u64 {
            entries.push(LeafEntry {
                key: BtreeKey::new(1, i),
                value: LeafValue::Inline(vec![Value::Int4(i as i32)]),
            });
        }
        assert!(leaf_entries_fit(&entries));
    }

    /// Every piece `split_points` cuts fits one page, for random entry lists whose
    /// entries each fit alone — leaves, and internal nodes whose cut entries are
    /// promoted — and a two-way cut is taken whenever one fits.
    #[test]
    fn split_points_cut_pieces_that_each_fit() {
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let mut below = |n: usize| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % n as u64) as usize
        };
        for round in 0..4000 {
            let promote = round % 2 == 1;
            let base = if promote { CHILD_SIZE } else { 0 };
            let max_entry = [60, 500, DATA_SIZE / 2, DATA_SIZE - base][round % 4];
            let n = 2 + below(60);
            let sizes: Vec<usize> = (0..n).map(|_| 20 + below(max_entry - 19)).collect();
            if base + sizes.iter().sum::<usize>() <= DATA_SIZE {
                continue;
            }
            let cuts = split_points(&sizes, base, promote);
            assert!(!cuts.is_empty(), "round {round}: an overfull node must cut");
            assert!(cuts.windows(2).all(|w| w[0] < w[1]), "round {round}");
            let mut start = 0;
            let mut pieces = Vec::new();
            for &cut in &cuts {
                pieces.push(start..cut);
                start = if promote { cut + 1 } else { cut };
            }
            pieces.push(start..n);
            for piece in &pieces {
                let bytes = base + sizes[piece.clone()].iter().sum::<usize>();
                assert!(
                    bytes <= DATA_SIZE,
                    "round {round}: piece {piece:?} of {sizes:?} is {bytes} bytes"
                );
                if !promote {
                    assert!(!piece.is_empty(), "round {round}: an empty leaf piece");
                }
            }
            // A two-way cut is preferred: if any single cut fits, only one is made.
            let any_two_way = (0..n).any(|i| {
                let (l, r) = if promote {
                    (&sizes[..i], &sizes[i + 1..])
                } else if i == 0 {
                    return false;
                } else {
                    (&sizes[..i], &sizes[i..])
                };
                base + l.iter().sum::<usize>() <= DATA_SIZE
                    && base + r.iter().sum::<usize>() <= DATA_SIZE
            });
            assert_eq!(cuts.len() == 1, any_two_way, "round {round}: {sizes:?}");
        }
    }

    /// `encoded_len` is exactly what `serialize` writes, so the in-memory store's
    /// budget check and the file store's writer refuse the same pages.
    #[test]
    fn encoded_len_is_what_serialize_writes() {
        let mut leaf = Page::new_leaf(3);
        if let Page::Leaf { entries, .. } = &mut leaf {
            let entries = Arc::make_mut(entries);
            for i in 0..5u64 {
                entries.push(LeafEntry {
                    key: BtreeKey::index(1, &vec![0u8; 10 * i as usize], i),
                    value: LeafValue::Inline(vec![
                        Value::Text("y".repeat(100 * i as usize)),
                        Value::Int8(7),
                    ]),
                });
            }
        }
        let bytes = leaf.serialize().unwrap();
        let used = bytes.len() - bytes.iter().rev().take_while(|&&b| b == 0).count();
        assert_eq!(leaf.encoded_len(), used);

        // One byte past a page: the writer refuses, it does not index past the buffer.
        let entry = |row_id: u64, len: usize| LeafEntry {
            key: BtreeKey::new(1, row_id),
            value: LeafValue::Inline(vec![Value::Bytea(vec![1; len])]),
        };
        let big = (0..200)
            .map(|len| {
                let mut page = Page::new_leaf(4);
                if let Page::Leaf { entries, .. } = &mut page {
                    *entries = Arc::new(vec![entry(1, 4000), entry(2, len)]);
                }
                page
            })
            .find(|page| page.encoded_len() == PAGE_SIZE + 1)
            .expect("some length lands one byte past a page");
        assert_eq!(big.encoded_len(), PAGE_SIZE + 1);
        assert!(matches!(
            big.serialize(),
            Err(PageStoreError::Overfull { bytes, .. }) if bytes == PAGE_SIZE + 1
        ));
    }

    /// A page whose bytes do not decode is `None`, never a panic: a truncated buffer,
    /// a key length past the page, and a value that is not postcard.
    #[test]
    fn deserialize_refuses_malformed_bytes() {
        let mut leaf = Page::new_leaf(1);
        if let Page::Leaf { entries, .. } = &mut leaf {
            Arc::make_mut(entries).push(LeafEntry {
                key: BtreeKey::new(1, 1),
                value: LeafValue::Inline(vec![Value::Int4(1)]),
            });
        }
        let bytes = leaf.serialize().unwrap();
        assert!(Page::deserialize(&bytes[..HEADER_SIZE + 3]).is_none());
        let mut bad_len = bytes.clone();
        bad_len[HEADER_SIZE..HEADER_SIZE + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Page::deserialize(&bad_len).is_none());
        let mut bad_value = bytes;
        // The value's postcard `Vec` length, claimed far past the four bytes present.
        let value_at = HEADER_SIZE + 4 + BtreeKey::DATA_SIZE + 4;
        bad_value[value_at] = 0x7F;
        assert!(Page::deserialize(&bad_value).is_none());
    }

    /// An overflow link and a leaf's out-of-line reference round-trip, and the decoder
    /// refuses a link with no payload or more than a page's, and a reference to an
    /// empty chain.
    #[test]
    fn overflow_pages_and_references_round_trip() {
        let link = Page::new_overflow(9, vec![7u8; DATA_SIZE], 10);
        let bytes = link.serialize().unwrap();
        let Some(Page::Overflow { header, data }) = Page::deserialize(&bytes) else {
            panic!("an overflow page must decode as one");
        };
        assert_eq!(header.next_or_reserved, 10);
        assert_eq!(*data, vec![7u8; DATA_SIZE]);

        let mut no_payload = bytes.clone();
        no_payload[10..12].copy_from_slice(&0u16.to_le_bytes());
        assert!(Page::deserialize(&no_payload).is_none());
        let mut too_long = bytes;
        too_long[10..12].copy_from_slice(&(DATA_SIZE as u16 + 1).to_le_bytes());
        assert!(Page::deserialize(&too_long).is_none());

        let r = OverflowRef { head: 9, len: 5000 };
        let mut leaf = Page::new_leaf(1);
        if let Page::Leaf { entries, .. } = &mut leaf {
            Arc::make_mut(entries).push(LeafEntry {
                key: BtreeKey::new(1, 1),
                value: LeafValue::Overflow(r),
            });
        }
        let bytes = leaf.serialize().unwrap();
        assert_eq!(
            leaf.encoded_len(),
            HEADER_SIZE + 4 + BtreeKey::DATA_SIZE + 4 + 16
        );
        let Some(Page::Leaf { entries, .. }) = Page::deserialize(&bytes) else {
            panic!("leaf");
        };
        assert_eq!(entries[0].value, LeafValue::Overflow(r));
        // The reference's length word is at the end of the entry: zero it.
        let mut empty = bytes;
        let len_at = HEADER_SIZE + 4 + BtreeKey::DATA_SIZE + 4 + 8;
        empty[len_at..len_at + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(Page::deserialize(&empty).is_none());
    }
}
