//! Power-loss images: record every write a store issues, then rebuild the image a
//! device could hold after losing power at any `sync` boundary.
//!
//! [`RecordingBlockIo`] is a faithful in-memory block device that also logs each
//! `write_at` / `set_len` / `sync` in issue order. A `sync` is the only ordering a
//! device promises: every write issued before it is on the medium once it returns, and
//! the writes issued since the last one may land in **any** subset, a torn page
//! included. [`power_loss_images`] enumerates that: at every boundary it builds the
//! durable prefix plus each [`Landing`] of the pending writes.
//!
//! [`reachable_pages`] is the audit side — the pages an image's adopted slot reaches
//! (tree pages and overflow links), walked with a visited set so a damaged image
//! cannot loop it — and [`DoubleMetaStore::persisted_chain_pages`] supplies the
//! free-list and retired-log chain pages both slots name.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use pglet_storage::btree::{
    BlockIo, DoubleMetaStore, LeafValue, NULL_PAGE_ID, Page, PageId, PageStore, PageStoreMetadata,
};

use crate::block_io::{DATA_REGION_START, MemBlockIo};
use crate::rng::Xorshift;

/// One operation a store issued to its block device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IoOp {
    /// A positioned write.
    Write {
        /// Byte offset.
        offset: u64,
        /// The bytes written.
        bytes: Vec<u8>,
    },
    /// A resize.
    SetLen(u64),
    /// An fsync.
    Sync,
}

impl IoOp {
    /// Whether this is a write into a meta slot (below the data region).
    #[must_use]
    pub const fn is_slot_write(&self) -> bool {
        matches!(self, Self::Write { offset, .. } if *offset < DATA_REGION_START)
    }
}

/// Apply `op` to `image` as a device that landed it whole would.
pub fn apply_op(image: &mut Vec<u8>, op: &IoOp) {
    match op {
        IoOp::Write { offset, bytes } => {
            let start = *offset as usize;
            let end = start + bytes.len();
            if image.len() < end {
                image.resize(end, 0);
            }
            image[start..end].copy_from_slice(bytes);
        }
        IoOp::SetLen(n) => image.resize(*n as usize, 0),
        IoOp::Sync => {}
    }
}

#[derive(Debug, Default)]
struct Recording {
    ops: Vec<IoOp>,
    live: Vec<u8>,
    /// While set, a meta-slot write fails with an I/O error and lands nothing.
    fail_slot_writes: bool,
}

/// A faithful in-memory `BlockIo` that logs every operation in issue order. Clones
/// share the log and the bytes, so a test keeps a handle after the store takes its
/// own.
#[derive(Clone, Debug, Default)]
pub struct RecordingBlockIo(Arc<Mutex<Recording>>);

impl RecordingBlockIo {
    /// An empty device.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Every operation issued so far.
    #[must_use]
    pub fn ops(&self) -> Vec<IoOp> {
        self.0.lock().unwrap().ops.clone()
    }

    /// How many operations have been issued so far.
    #[must_use]
    pub fn op_count(&self) -> usize {
        self.0.lock().unwrap().ops.len()
    }

    /// The bytes as every issued write left them.
    #[must_use]
    pub fn image(&self) -> Vec<u8> {
        self.0.lock().unwrap().live.clone()
    }

    /// While `fail` is set, every meta-slot write returns an I/O error and lands
    /// nothing — a commit whose durable point fails.
    pub fn fail_slot_writes(&self, fail: bool) {
        self.0.lock().unwrap().fail_slot_writes = fail;
    }
}

impl BlockIo for RecordingBlockIo {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let r = self.0.lock().unwrap();
        let start = offset as usize;
        if start >= r.live.len() {
            return Ok(0);
        }
        let end = (start + buf.len()).min(r.live.len());
        buf[..end - start].copy_from_slice(&r.live[start..end]);
        Ok(end - start)
    }
    fn write_at(&self, offset: u64, buf: &[u8]) -> std::io::Result<()> {
        let mut r = self.0.lock().unwrap();
        if r.fail_slot_writes && offset < DATA_REGION_START {
            return Err(std::io::Error::other("injected meta-slot write failure"));
        }
        let op = IoOp::Write {
            offset,
            bytes: buf.to_vec(),
        };
        apply_op(&mut r.live, &op);
        r.ops.push(op);
        Ok(())
    }
    fn sync(&self) -> std::io::Result<()> {
        self.0.lock().unwrap().ops.push(IoOp::Sync);
        Ok(())
    }
    fn len(&self) -> std::io::Result<u64> {
        Ok(self.0.lock().unwrap().live.len() as u64)
    }
    fn set_len(&self, size: u64) -> std::io::Result<()> {
        let mut r = self.0.lock().unwrap();
        let op = IoOp::SetLen(size);
        apply_op(&mut r.live, &op);
        r.ops.push(op);
        Ok(())
    }
}

/// Which of the writes pending at a boundary land.
#[derive(Clone, Debug)]
pub enum Landing {
    /// Every pending write lands whole — what a `kill -9` leaves.
    All,
    /// None does.
    None,
    /// Only the meta-slot writes land.
    SlotOnly,
    /// Only the data-region writes land.
    DataOnly,
    /// A seeded subset lands, and one write (when `torn` names it) lands only its
    /// first `cut` bytes.
    Subset {
        /// Per pending write, whether it lands.
        mask: Vec<bool>,
        /// A pending write's index and the prefix of it that lands.
        torn: Option<(usize, usize)>,
    },
}

/// One power-loss image.
#[derive(Clone, Debug)]
pub struct PowerLossImage {
    /// The index into the op log of the `sync` (or the log's end) the power was lost
    /// before.
    pub boundary: usize,
    /// The index into the op log of the last `sync` whose writes are all durable (0
    /// when none is).
    pub durable_through: usize,
    /// A name for the landing, for a failure message.
    pub landing: String,
    /// The bytes.
    pub image: Vec<u8>,
}

/// Every power-loss image of `ops`.
///
/// At each `sync` boundary (and the log's end): the durable prefix plus each landing of
/// the writes pending since the previous `sync` — all, none, slot-only, data-only, and
/// `subsets` seeded random subsets, every other one with a write torn at a 512-byte
/// boundary.
#[must_use]
pub fn power_loss_images(ops: &[IoOp], subsets: usize, seed: u64) -> Vec<PowerLossImage> {
    let mut rng = Xorshift::new(seed | 1);
    let mut out = Vec::new();
    let mut durable: Vec<u8> = Vec::new();
    let mut last_sync = 0usize;
    let mut boundaries: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| **op == IoOp::Sync)
        .map(|(i, _)| i)
        .collect();
    boundaries.push(ops.len());
    for boundary in boundaries {
        let pending: Vec<usize> = (last_sync..boundary)
            .filter(|&i| ops[i] != IoOp::Sync)
            .collect();
        let mut landings = vec![
            ("all".to_string(), Landing::All),
            ("none".to_string(), Landing::None),
            ("slot-only".to_string(), Landing::SlotOnly),
            ("data-only".to_string(), Landing::DataOnly),
        ];
        for k in 0..subsets {
            let mask: Vec<bool> = pending.iter().map(|_| rng.below(2) == 0).collect();
            let torn = (!pending.is_empty() && k % 2 == 1).then(|| {
                let j = rng.below(pending.len() as u64) as usize;
                (j, rng.below(8) as usize * 512)
            });
            landings.push((format!("subset{k}"), Landing::Subset { mask, torn }));
        }
        for (name, landing) in landings {
            let mut image = durable.clone();
            for (pi, &i) in pending.iter().enumerate() {
                let op = &ops[i];
                let lands = match &landing {
                    Landing::All => true,
                    Landing::None => false,
                    Landing::SlotOnly => op.is_slot_write(),
                    Landing::DataOnly => !op.is_slot_write(),
                    Landing::Subset { mask, torn } => {
                        if let (Some((tj, cut)), IoOp::Write { offset, bytes }) = (torn, op)
                            && *tj == pi
                        {
                            let cut = (*cut).min(bytes.len());
                            apply_op(
                                &mut image,
                                &IoOp::Write {
                                    offset: *offset,
                                    bytes: bytes[..cut].to_vec(),
                                },
                            );
                            continue;
                        }
                        mask[pi]
                    }
                };
                if lands {
                    apply_op(&mut image, op);
                }
            }
            out.push(PowerLossImage {
                boundary,
                durable_through: last_sync,
                landing: name,
                image,
            });
        }
        for &i in &pending {
            apply_op(&mut durable, &ops[i]);
        }
        last_sync = boundary;
    }
    out
}

/// What an image's adopted slot reaches, read through the store's own open.
#[derive(Clone, Debug)]
pub struct ImageAudit {
    /// The adopted slot.
    pub metadata: PageStoreMetadata,
    /// Every tree page and overflow link the adopted root reaches.
    pub live: BTreeSet<PageId>,
    /// The free list open recovered.
    pub free: Vec<PageId>,
    /// The free-list and retired-log chain pages open recorded for both slots.
    pub chains: Vec<PageId>,
}

impl ImageAudit {
    /// Recovered free ids the adopted tree reaches.
    #[must_use]
    pub fn live_on_free_list(&self) -> Vec<PageId> {
        self.free
            .iter()
            .copied()
            .filter(|id| self.live.contains(id))
            .collect()
    }

    /// Recovered free ids listed more than once, or also held as a chain page.
    #[must_use]
    pub fn free_list_aliases(&self) -> Vec<PageId> {
        let mut seen = BTreeSet::new();
        let chains: BTreeSet<PageId> = self.chains.iter().copied().collect();
        self.free
            .iter()
            .copied()
            .filter(|id| !seen.insert(*id) || chains.contains(id))
            .collect()
    }

    /// Pages below `next_page_id` that are neither live, free, nor a chain page:
    /// allocated ids nothing will ever hand out again.
    #[must_use]
    pub fn leaked(&self) -> Vec<PageId> {
        let free: BTreeSet<PageId> = self.free.iter().copied().collect();
        let chains: BTreeSet<PageId> = self.chains.iter().copied().collect();
        (0..self.metadata.next_page_id)
            .filter(|id| !self.live.contains(id) && !free.contains(id) && !chains.contains(id))
            .collect()
    }
}

/// Open `image` as the store opens it and walk what its adopted slot reaches. The walk
/// keeps a visited set, so a damaged image cannot loop it.
///
/// # Errors
///
/// The store's open error, or the first page the walk could not read.
pub fn audit_image(image: &[u8]) -> Result<ImageAudit, String> {
    let (store, metadata) = DoubleMetaStore::open(Box::new(MemBlockIo::from_bytes(image.to_vec())))
        .map_err(|e| format!("open: {e}"))?;
    let live = reachable_pages(&store, metadata.root)?;
    Ok(ImageAudit {
        metadata,
        live,
        free: store.free_list(),
        chains: store.persisted_chain_pages().concat(),
    })
}

/// Every tree page and overflow link `root` reaches in `store`, each visited once.
///
/// # Errors
///
/// The first page the walk could not read, or a page of the wrong kind.
pub fn reachable_pages(store: &dyn PageStore, root: PageId) -> Result<BTreeSet<PageId>, String> {
    let mut live = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if id == NULL_PAGE_ID || !live.insert(id) {
            continue;
        }
        match store.get(id) {
            Ok(Some(Page::Internal {
                first_child,
                entries,
                ..
            })) => {
                stack.push(first_child);
                stack.extend(entries.iter().map(|e| e.child));
            }
            Ok(Some(Page::Leaf { entries, .. })) => {
                for entry in entries.iter() {
                    let LeafValue::Overflow(r) = entry.value else {
                        continue;
                    };
                    let mut link = r.head;
                    for _ in 0..r.page_count() {
                        if link == NULL_PAGE_ID || !live.insert(link) {
                            break;
                        }
                        match store.get(link) {
                            Ok(Some(Page::Overflow { header, .. })) => {
                                link = header.next_or_reserved;
                            }
                            other => return Err(format!("overflow link {link}: {other:?}")),
                        }
                    }
                }
            }
            other => return Err(format!("tree page {id}: {other:?}")),
        }
    }
    Ok(live)
}
