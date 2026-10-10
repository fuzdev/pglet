//! Sharded write-through page cache for [`DoubleMetaStore`](super::double_meta::DoubleMetaStore).
//!
//! The store's read path is otherwise lock-free — a reader captures the atomic
//! root and traverses an immutable CoW tree with no locks. The page cache was the
//! one serialization point left: a single global `Mutex<LruCache>` taken on every
//! `get`, so concurrent readers serialized *per page read* despite the lock-free
//! snapshot. An `LruCache` needs `&mut` per `get` (it bumps recency), so it can't
//! be a plain `RwLock`.
//!
//! This shards the cache into `N` independent `Mutex<LruCache>` lanes keyed by a
//! hash of the page id, so concurrent readers contend only when two reads hash to
//! the same shard — cutting contention ~`N`-fold. Each shard keeps its own LRU
//! recency (a slightly weaker *global* LRU than one big cache, which is fine for a
//! page cache; the hot set lands in whichever shards own it).
//!
//! # Correctness
//!
//! All three operations — [`get`](ShardedPageCache::get),
//! [`put`](ShardedPageCache::put), [`pop`](ShardedPageCache::pop) — route through
//! the **one** [`shard_index`](ShardedPageCache::shard_index) function, so they
//! always agree on which shard owns a page id. This is load-bearing for the
//! page-id-reuse invariant: CoW reuses freed page ids, so the store pops each
//! freed id from the cache before it can be reallocated — and that pop must land
//! in the same shard a later `get` reads, or a stale page under a reused id would
//! be silent corruption. A single source of truth for the routing makes that
//! impossible to get subtly wrong.
//!
//! # Hashing
//!
//! The shard is chosen by a [Fibonacci hash](https://en.wikipedia.org/wiki/Hash_function#Fibonacci_hashing)
//! (multiply by the golden-ratio constant, take the high bits) rather than
//! `page_id % N`. Page ids are far from random — sequential allocation, LIFO
//! free-list reuse, and a hot working set of upper-tree pages would each cluster
//! under a low-bits modulo. The multiply mixes all bits into the high ones, so any
//! structured id pattern disperses evenly across shards.

use std::num::NonZeroUsize;

use lru::LruCache;
use parking_lot::Mutex;

use super::page::{Page, PageId};

/// Number of cache shards. A power of two so the Fibonacci-hash shard index is a
/// single right-shift (no modulo). 16 comfortably exceeds the concurrent-reader
/// counts this store sees (the thread-per-connection server; differential/test
/// workloads) while keeping per-shard LRU recency meaningful.
const DEFAULT_SHARD_COUNT: usize = 16;

/// The golden-ratio constant for 64-bit Fibonacci hashing (`2^64 / φ`, odd).
const FIBONACCI_HASH_MULTIPLIER: u64 = 0x9E37_79B9_7F4A_7C15;

/// A write-through page cache sharded into independent `Mutex<LruCache>` lanes.
///
/// Replaces the single global cache mutex so concurrent `get`s contend only when
/// they hash to the same shard. See the module docs for the routing-correctness
/// and hashing rationale.
pub(super) struct ShardedPageCache {
    /// One independent LRU lane per shard. Boxed slice: fixed length after
    /// construction, one heap indirection.
    shards: Box<[Mutex<LruCache<PageId, Page>>]>,
    /// `64 - log2(shards.len())` — the Fibonacci-hash right-shift selecting a
    /// shard from the high (well-mixed) bits. Unused when there is a single shard
    /// (a shift of 64 would be undefined), which [`shard_index`](Self::shard_index)
    /// special-cases.
    shard_shift: u32,
}

impl ShardedPageCache {
    /// Build a sharded cache whose shards' capacities sum to exactly
    /// `total_capacity` pages (clamped to at least 1).
    ///
    /// The capacity is carved across [`DEFAULT_SHARD_COUNT`] shards, with the
    /// remainder spread one-per-shard so the totals add up and every shard holds
    /// at least one page. A `total_capacity` smaller than the shard count uses
    /// fewer shards (the largest power of two `<= total_capacity`), so a tiny cache
    /// (the "pass a tiny value to effectively disable caching" measurement path)
    /// collapses toward a single lane rather than spreading one page across many
    /// idle shards.
    pub(super) fn new(total_capacity: usize) -> Self {
        let cap = total_capacity.max(1);
        // Largest power of two that is both <= cap (so every shard gets >= 1 page)
        // and <= DEFAULT_SHARD_COUNT. `cap >= 1`, so `ilog2` never panics.
        let shard_count = (1usize << cap.ilog2()).min(DEFAULT_SHARD_COUNT);
        let shard_shift = u64::BITS - shard_count.trailing_zeros();

        // Carve `cap` across the shards: the first `remainder` shards get one extra
        // page so the per-shard caps sum to exactly `cap`. `shard_count <= cap`, so
        // `base >= 1` and every shard capacity is non-zero.
        let base = cap / shard_count;
        let remainder = cap % shard_count;
        let shards = (0..shard_count)
            .map(|i| {
                let per = base + usize::from(i < remainder);
                Mutex::new(LruCache::new(
                    NonZeroUsize::new(per).unwrap_or(NonZeroUsize::MIN),
                ))
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            shards,
            shard_shift,
        }
    }

    /// Map a page id to its shard via a Fibonacci hash of the id (high bits of the
    /// golden-ratio product), so structured ids disperse instead of clustering.
    #[inline]
    fn shard_index(&self, page_id: PageId) -> usize {
        if self.shards.len() == 1 {
            return 0;
        }
        let hash = page_id.wrapping_mul(FIBONACCI_HASH_MULTIPLIER);
        // The shift keeps exactly `log2(shard_count)` high bits, so the result is
        // already in `0..shard_count` — no mask, and never wider than `usize`.
        #[allow(clippy::cast_possible_truncation)]
        let index = (hash >> self.shard_shift) as usize;
        index
    }

    /// Look up a page, bumping its LRU recency. Returns a clone (pages are
    /// immutable by id between alloc and free, so a clone needs no further
    /// coordination).
    #[inline]
    pub(super) fn get(&self, page_id: PageId) -> Option<Page> {
        self.shards[self.shard_index(page_id)]
            .lock()
            .get(&page_id)
            .cloned()
    }

    /// Insert (or refresh) a page. Used write-through by `put` and to populate the
    /// cache on a read miss.
    #[inline]
    pub(super) fn put(&self, page_id: PageId, page: Page) {
        self.shards[self.shard_index(page_id)]
            .lock()
            .put(page_id, page);
    }

    /// Evict a page id from its shard. The store calls this for every freed id so a
    /// reused id can never serve a stale cached page.
    #[inline]
    pub(super) fn pop(&self, page_id: PageId) {
        self.shards[self.shard_index(page_id)].lock().pop(&page_id);
    }

    /// Total number of cached pages across all shards (for diagnostics / `Debug`).
    pub(super) fn len(&self) -> usize {
        self.shards.iter().map(|s| s.lock().len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btree::page::Page;

    #[test]
    fn routes_get_put_pop_to_the_same_shard() {
        // If `get`/`put`/`pop` ever disagreed on the shard for an id, a put would
        // land where a later get can't see it (miss), or a pop wouldn't evict where
        // get reads (stale). Round-tripping a spread of ids proves they agree.
        let cache = ShardedPageCache::new(2000);
        for id in 0..1000u64 {
            cache.put(id, Page::new_leaf(id));
        }
        for id in 0..1000u64 {
            let page = cache.get(id).expect("a just-put page must be gettable");
            assert_eq!(
                page.page_id(),
                id,
                "get returned a page from the wrong shard"
            );
        }
        // Pop routes to the same shard get reads — so the eviction is observable.
        for id in 0..1000u64 {
            cache.pop(id);
            assert!(
                cache.get(id).is_none(),
                "pop did not evict from get's shard"
            );
        }
    }

    #[test]
    fn disperses_ids_across_shards() {
        // The Fibonacci hash must spread *sequential* ids (the clustering case a
        // low-bits modulo would fail) across many shards, not pile them into one.
        let cache = ShardedPageCache::new(2000);
        for id in 0..1000u64 {
            cache.put(id, Page::new_leaf(id));
        }
        let used = cache.shards.iter().filter(|s| !s.lock().is_empty()).count();
        assert_eq!(
            used, DEFAULT_SHARD_COUNT,
            "1000 sequential ids should populate every shard"
        );
    }

    #[test]
    fn capacity_is_carved_exactly_across_shards() {
        // Per-shard caps must sum to the requested total (no over- or
        // under-allocation of the page budget).
        let cache = ShardedPageCache::new(2000);
        let total: usize = cache.shards.iter().map(|s| s.lock().cap().get()).sum();
        assert_eq!(total, 2000);
        assert_eq!(cache.shards.len(), DEFAULT_SHARD_COUNT);
    }

    #[test]
    fn capacity_carving_holds_across_sizes() {
        // The carving invariants must hold for *any* size, not just the two fixed
        // cases above — guarding the `ilog2`/remainder math against off-by-one at
        // the power-of-two boundaries (15/16/17, 31, …): shard count is a power of
        // two, never exceeds min(cap, DEFAULT_SHARD_COUNT), and the per-shard caps
        // sum to exactly the requested capacity (clamped to >= 1) with no zero shard.
        for &size in &[0usize, 1, 2, 3, 7, 15, 16, 17, 31, 125, 999, 2000, 100_000] {
            let cache = ShardedPageCache::new(size);
            let cap = size.max(1);
            let n = cache.shards.len();
            assert!(
                n.is_power_of_two(),
                "size {size}: shard count {n} not a power of two"
            );
            assert!(
                n <= DEFAULT_SHARD_COUNT,
                "size {size}: {n} shards exceeds the cap"
            );
            assert!(
                n <= cap,
                "size {size}: more shards ({n}) than pages ({cap})"
            );
            let caps: Vec<usize> = cache.shards.iter().map(|s| s.lock().cap().get()).collect();
            assert!(
                caps.iter().all(|&c| c >= 1),
                "size {size}: a shard has zero capacity"
            );
            assert_eq!(
                caps.iter().sum::<usize>(),
                cap,
                "size {size}: per-shard caps must sum to the total"
            );
        }
    }

    #[test]
    fn tiny_capacity_collapses_to_a_single_shard() {
        // The disable-caching measurement path (cap 1) must not spread one page
        // across 16 idle shards — it collapses to one lane that holds one page.
        let cache = ShardedPageCache::new(1);
        assert_eq!(cache.shards.len(), 1);
        let total: usize = cache.shards.iter().map(|s| s.lock().cap().get()).sum();
        assert_eq!(total, 1);
        // Single-shard routing is the degenerate index-0 case.
        cache.put(42, Page::new_leaf(42));
        assert_eq!(cache.get(42).map(|p| p.page_id()), Some(42));
    }
}
