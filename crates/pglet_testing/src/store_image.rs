//! Editing a double-meta store image by hand, behind valid checksums.
//!
//! A raw byte flip in a data page fails that page's checksum before the page codec or
//! the tree reads a byte of it, so a corruption test built from flips alone guards
//! the checksum and nothing past it. These helpers edit a page and re-stamp its
//! checksum with the store's own rule (`DoubleMetaStore::stamp_page_checksum`), so
//! the damage reaches the codec, the tree, and the overflow-chain walk.
//!
//! [`ChainDamage`] names each shape of a damaged overflow chain the chain walk must
//! refuse, and [`ChainDamage::apply`] writes it into an image; the store-level twin
//! of that list is `pglet_storage`'s own `a_damaged_chain_is_a_typed_corruption_error`
//! unit test, which cannot share this code (its in-crate test build is a distinct
//! instance of `pglet_storage`).

use std::ops::Range;
use std::sync::Arc;

use pglet_storage::btree::{
    BtreeKey, DATA_SIZE, DoubleMetaStore, LeafValue, NULL_PAGE_ID, OverflowRef, PAGE_SIZE, Page,
    PageId, PageStore,
};

use crate::block_io::{DATA_REGION_START, MemBlockIo};

/// The byte range page `id` occupies in an image.
#[must_use]
pub const fn page_range(id: PageId) -> Range<usize> {
    let start = DATA_REGION_START as usize + id as usize * PAGE_SIZE;
    start..start + PAGE_SIZE
}

/// The ids of the whole pages an image holds.
#[must_use]
pub const fn page_ids(image: &[u8]) -> Range<PageId> {
    let data = image.len().saturating_sub(DATA_REGION_START as usize);
    0..(data / PAGE_SIZE) as PageId
}

/// Page `id` as the image holds it, decoded without its checksum being asked; `None`
/// if the image does not hold the page or its bytes do not decode.
#[must_use]
pub fn read_page(image: &[u8], id: PageId) -> Option<Page> {
    Page::deserialize(image.get(page_range(id))?)
}

/// Re-stamp page `id`'s checksum over whatever bytes it now holds.
pub fn restamp(image: &mut [u8], id: PageId) {
    let page: &mut [u8; PAGE_SIZE] = (&mut image[page_range(id)])
        .try_into()
        .expect("a whole page");
    DoubleMetaStore::stamp_page_checksum(page);
}

/// Write `page` into the image at its own id, serialized and checksum-stamped as the
/// store writes it.
pub fn write_page(image: &mut [u8], page: &Page) {
    let id = page.page_id();
    let bytes = page.serialize().expect("an edited page still fits");
    image[page_range(id)].copy_from_slice(&bytes);
    restamp(image, id);
}

/// An overflow chain the committed tree reaches: the key whose leaf entry references
/// it, that leaf, the reference, and the links as the chain itself links them.
#[derive(Clone, Debug)]
pub struct StoreChain {
    /// The entry's key.
    pub key: BtreeKey,
    /// The leaf page holding the entry.
    pub leaf: PageId,
    /// The entry's reference to the chain.
    pub reference: OverflowRef,
    /// The chain's links, head first.
    pub links: Vec<PageId>,
}

/// Every overflow chain the image's committed tree reaches, and the store's
/// `next_page_id` — read through the store's own open, so what is found is what a
/// reopen reads.
///
/// # Panics
///
/// If the image does not open, or its committed tree does not read back.
#[must_use]
pub fn reachable_chains(image: &[u8]) -> (PageId, Vec<StoreChain>) {
    let (store, meta) =
        DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(image.to_vec()))).expect("opens");
    let mut chains = Vec::new();
    let mut stack = vec![meta.root];
    while let Some(id) = stack.pop() {
        if id == NULL_PAGE_ID {
            continue;
        }
        match store.get(id).expect("reads").expect("present") {
            Page::Internal {
                first_child,
                entries,
                ..
            } => {
                stack.push(first_child);
                stack.extend(entries.iter().map(|e| e.child));
            }
            Page::Leaf { entries, .. } => {
                for entry in entries.iter() {
                    let LeafValue::Overflow(reference) = entry.value else {
                        continue;
                    };
                    let mut links = vec![reference.head];
                    while (links.len() as u64) < reference.page_count() {
                        let Some(Page::Overflow { header, .. }) =
                            store.get(*links.last().expect("a head")).expect("reads")
                        else {
                            panic!("chain from {} is not whole", reference.head);
                        };
                        links.push(header.next_or_reserved);
                    }
                    chains.push(StoreChain {
                        key: entry.key.clone(),
                        leaf: id,
                        reference,
                        links,
                    });
                }
            }
            Page::Free { .. } | Page::Overflow { .. } => panic!("page {id} is not a tree page"),
        }
    }
    (meta.next_page_id, chains)
}

/// A shape of damaged overflow chain — each one the chain walk must refuse as a
/// corrupt chain, naming the page where the chain stopped agreeing with its
/// reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainDamage {
    /// The head links on to a page id the store never allocated.
    LinkPastEnd,
    /// The head links on to the highest page id short of the null link — past what
    /// the store allocated, and past where a page offset can be computed.
    LinkAtTopOfIdSpace,
    /// A link is replaced by a page of another kind (an empty leaf).
    WrongKind,
    /// The second link (the head, in a one-link chain) carries one byte less than it
    /// should — not the head where there is a second, since a short head and a failed
    /// decode of the whole value would name the same page.
    ShortLink,
    /// The last link links on, back to the head.
    GoesOn,
    /// The link before the last ends the chain.
    EndsEarly,
    /// The second link loops back to the head.
    LoopsBack,
    /// The reference claims one page where the chain has more.
    ReferenceShort,
    /// The reference claims one page more than the store has ever allocated.
    ReferencePastStore,
    /// The head links to itself under a reference as long as the store.
    LoopAsLongAsStore,
    /// The head links to itself under a reference claiming a terabyte.
    LoopClaimingTerabyte,
}

impl ChainDamage {
    /// Every shape.
    pub const ALL: [Self; 11] = [
        Self::LinkPastEnd,
        Self::LinkAtTopOfIdSpace,
        Self::WrongKind,
        Self::ShortLink,
        Self::GoesOn,
        Self::EndsEarly,
        Self::LoopsBack,
        Self::ReferenceShort,
        Self::ReferencePastStore,
        Self::LoopAsLongAsStore,
        Self::LoopClaimingTerabyte,
    ];

    /// Write this damage into `image` at `chain`, returning the page the refusal must
    /// name — or `None`, leaving the image untouched, when the chain has too few
    /// links for the shape.
    ///
    /// # Panics
    ///
    /// If `chain` is not a chain `image` holds.
    pub fn apply(
        self,
        image: &mut [u8],
        chain: &StoreChain,
        next_page_id: PageId,
    ) -> Option<PageId> {
        let links = &chain.links;
        let n = links.len();
        let head = links[0];
        let relink = |image: &mut [u8], id: PageId, next: PageId, cut: usize| {
            let Some(Page::Overflow { data, .. }) = read_page(image, id) else {
                panic!("link {id}");
            };
            let data = data[..data.len() - cut].to_vec();
            write_page(image, &Page::new_overflow(id, data, next));
        };
        let refer = |image: &mut [u8], len: u64| {
            let Some(mut leaf) = read_page(image, chain.leaf) else {
                panic!("leaf {}", chain.leaf);
            };
            if let Page::Leaf { entries, .. } = &mut leaf {
                let entries = Arc::make_mut(entries);
                let entry = entries
                    .iter_mut()
                    .find(|e| e.key == chain.key)
                    .expect("the entry");
                entry.value = LeafValue::Overflow(OverflowRef { head, len });
            }
            write_page(image, &leaf);
        };
        let full = DATA_SIZE as u64;
        Some(match self {
            Self::LinkPastEnd => {
                let past = next_page_id + 1_000;
                relink(image, head, past, 0);
                if n > 1 { past } else { head }
            }
            Self::LinkAtTopOfIdSpace => {
                let top = NULL_PAGE_ID - 1;
                relink(image, head, top, 0);
                if n > 1 { top } else { head }
            }
            Self::WrongKind => {
                let id = links[1.min(n - 1)];
                write_page(image, &Page::new_leaf(id));
                id
            }
            Self::ShortLink => {
                let at = 1.min(n - 1);
                let next = links.get(at + 1).copied().unwrap_or(NULL_PAGE_ID);
                relink(image, links[at], next, 1);
                links[at]
            }
            Self::GoesOn => {
                relink(image, links[n - 1], head, 0);
                links[n - 1]
            }
            Self::EndsEarly if n >= 2 => {
                relink(image, links[n - 2], NULL_PAGE_ID, 0);
                links[n - 2]
            }
            Self::LoopsBack if n >= 3 => {
                relink(image, links[1], head, 0);
                head
            }
            Self::ReferenceShort if n >= 2 => {
                refer(image, full);
                head
            }
            Self::ReferencePastStore => {
                refer(image, (next_page_id + 1) * full);
                head
            }
            Self::LoopAsLongAsStore => {
                refer(image, next_page_id * full);
                relink(image, head, head, 0);
                head
            }
            Self::LoopClaimingTerabyte => {
                refer(image, 1 << 40);
                relink(image, head, head, 0);
                head
            }
            Self::EndsEarly | Self::LoopsBack | Self::ReferenceShort => return None,
        })
    }
}

/// The error text the chain walk refuses a chain with, as it reaches a caller that
/// renders the error: `head` is the chain's first page, `page` where it stopped
/// agreeing with its reference.
#[must_use]
pub fn corrupt_chain_message(head: PageId, page: PageId) -> String {
    format!("overflow chain from page {head} is damaged at page {page} (corrupt store)")
}

/// Whether page `id`'s header names it a leaf or an overflow page, as its bytes now
/// read.
#[must_use]
pub fn is_leaf_or_overflow(image: &[u8], id: PageId) -> bool {
    let at = page_range(id).start;
    image
        .get(at + 8..at + 10)
        .is_some_and(|flags| matches!(u16::from_le_bytes([flags[0], flags[1]]), 2 | 3))
}

/// Re-stamp the checksum of every whole page whose header, as its bytes now read,
/// names it a leaf or an overflow page, so damage written into those pages reaches
/// the leaf codec and the chain walk.
///
/// Internal pages keep the checksum their damage fails: the helper is aimed at the leaf
/// and chain structure. A tree walk refuses a page it reaches twice — a child naming
/// an ancestor included — as a corrupt tree, so re-stamping internal pages too would
/// reach that check rather than hang a read; no caller does yet.
pub fn restamp_leaf_and_overflow_pages(image: &mut [u8]) {
    for id in page_ids(image) {
        if is_leaf_or_overflow(image, id) {
            restamp(image, id);
        }
    }
}
