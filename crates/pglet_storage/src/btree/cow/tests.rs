use super::*;
use crate::btree::PAGE_SIZE;

#[test]
fn test_empty_tree() {
    let tree = CowBtree::new();
    assert!(tree.is_empty());
    assert_eq!(tree.current_version(), 0);
}

#[test]
fn test_insert_single() {
    let tree = CowBtree::new();
    let key = BtreeKey::new(1, 100);
    let value = vec![Value::Int4(42)];

    tree.insert(key.clone(), value.clone()).unwrap();

    assert!(!tree.is_empty());
    assert_eq!(tree.current_version(), 1);
    let result = tree.get(&key).unwrap();
    assert_eq!(result, Some(value));
}

#[test]
fn test_insert_multiple() {
    let tree = CowBtree::new();

    for i in 0..10 {
        let key = BtreeKey::new(1, i);
        tree.insert(key, vec![Value::Int4(i as i32)]).unwrap();
    }

    assert_eq!(tree.current_version(), 10);

    for i in 0..10 {
        let key = BtreeKey::new(1, i);
        let result = tree.get(&key).unwrap();
        assert_eq!(result, Some(vec![Value::Int4(i as i32)]));
    }
}

#[test]
fn test_snapshot_isolation() {
    let tree = CowBtree::new();

    // Insert initial data
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 2), vec![Value::Int4(2)])
        .unwrap();

    // Take snapshot
    let snapshot = tree.snapshot();
    assert_eq!(snapshot.version, 2);

    // Insert more data
    tree.insert(BtreeKey::new(1, 3), vec![Value::Int4(3)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 4), vec![Value::Int4(4)])
        .unwrap();

    // Current view sees all 4 rows
    let current_results = tree.scan_table(1).unwrap();
    assert_eq!(current_results.len(), 4);

    // Snapshot view sees only 2 rows
    let snapshot_results = tree.scan_table_at_root(snapshot.root, 1).unwrap();
    assert_eq!(snapshot_results.len(), 2);

    // Release snapshot
    tree.release_snapshot(&snapshot);
}

#[test]
fn test_delete() {
    let tree = CowBtree::new();
    let key = BtreeKey::new(1, 100);
    let value = vec![Value::Int4(42)];

    tree.insert(key.clone(), value.clone()).unwrap();
    let deleted = tree.delete(&key).unwrap();
    assert_eq!(deleted, Some(value));

    let result = tree.get(&key).unwrap();
    assert_eq!(result, None);
}

#[test]
fn test_delete_with_snapshot() {
    let tree = CowBtree::new();

    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 2), vec![Value::Int4(2)])
        .unwrap();

    let snapshot = tree.snapshot();

    tree.delete(&BtreeKey::new(1, 1)).unwrap();

    // Current view: only row 2
    let current = tree.scan_table(1).unwrap();
    assert_eq!(current.len(), 1);

    // Snapshot view: both rows
    let snap_results = tree.scan_table_at_root(snapshot.root, 1).unwrap();
    assert_eq!(snap_results.len(), 2);

    tree.release_snapshot(&snapshot);
}

#[test]
fn test_version_increments() {
    let tree = CowBtree::new();
    assert_eq!(tree.current_version(), 0);

    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();
    assert_eq!(tree.current_version(), 1);

    tree.insert(BtreeKey::new(1, 2), vec![Value::Int4(2)])
        .unwrap();
    assert_eq!(tree.current_version(), 2);

    tree.delete(&BtreeKey::new(1, 1)).unwrap();
    assert_eq!(tree.current_version(), 3);
}

#[test]
fn test_cow_creates_new_pages() {
    let tree = CowBtree::new();

    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();
    let stats1 = tree.stats();

    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(2)])
        .unwrap();
    let stats2 = tree.stats();

    // Updating same key should create new page (CoW)
    assert!(stats2.page_count >= stats1.page_count);
    assert_ne!(stats1.root, stats2.root);
}

#[test]
fn test_insert_causes_split() {
    let tree = CowBtree::new();

    for i in 0..100 {
        tree.insert(BtreeKey::new(1, i), vec![Value::Int4(i as i32)])
            .unwrap();
    }

    let stats = tree.stats();
    assert!(stats.page_count > 1);

    // Verify all keys accessible
    for i in 0..100 {
        let result = tree.get(&BtreeKey::new(1, i)).unwrap();
        assert_eq!(result, Some(vec![Value::Int4(i as i32)]));
    }
}

#[test]
fn test_concurrent_read_during_write() {
    use std::sync::Arc;
    use std::thread;

    let tree = Arc::new(CowBtree::new());

    // Insert initial data
    for i in 0..10 {
        tree.insert(BtreeKey::new(1, i), vec![Value::Int4(i as i32)])
            .unwrap();
    }

    let snapshot = tree.snapshot();
    let tree_clone = Arc::clone(&tree);
    let snapshot_clone = snapshot.clone();

    // Writer thread
    let writer = thread::spawn(move || {
        for i in 10..20 {
            tree_clone
                .insert(BtreeKey::new(1, i), vec![Value::Int4(i as i32)])
                .unwrap();
        }
    });

    // Reader can continue using snapshot
    for _ in 0..5 {
        let results = tree.scan_table_at_root(snapshot_clone.root, 1).unwrap();
        assert_eq!(results.len(), 10); // Original 10 rows
    }

    writer.join().unwrap();

    // After writer finishes, current view has all 20
    let final_results = tree.scan_table(1).unwrap();
    assert_eq!(final_results.len(), 20);

    tree.release_snapshot(&snapshot);
}

#[test]
fn test_multiple_tables() {
    let tree = CowBtree::new();

    for table in 1..=3 {
        for row in 0..10 {
            tree.insert(
                BtreeKey::new(table, row),
                vec![Value::Int4((table * 100 + row as u32) as i32)],
            )
            .unwrap();
        }
    }

    for table in 1..=3 {
        let results = tree.scan_table(table).unwrap();
        assert_eq!(results.len(), 10);
        for (key, _) in results {
            assert_eq!(key.table_id(), Some(table));
        }
    }
}

// ========================================================================
// Vacuum / GC Tests
// ========================================================================

#[test]
fn test_vacuum_without_snapshots() {
    let tree = CowBtree::new();

    // Insert data to create pages
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 2), vec![Value::Int4(2)])
        .unwrap();

    // Update to create old pages
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(10)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 2), vec![Value::Int4(20)])
        .unwrap();

    let stats_before = tree.stats();
    assert!(
        stats_before.pending_gc_pages > 0,
        "should have pending GC pages"
    );

    // Run vacuum - should free all pending pages (no active snapshots)
    let result = tree.vacuum();
    assert!(result.pages_freed > 0, "should free pages");
    assert_eq!(result.pages_pending, 0, "no pages should be pending");
    assert!(result.min_active_version.is_none(), "no active snapshots");

    let stats_after = tree.stats();
    assert_eq!(stats_after.pending_gc_pages, 0, "all pages should be freed");
    assert!(
        stats_after.free_list_size > 0,
        "freed pages should be in free list"
    );
}

#[test]
fn test_vacuum_respects_active_snapshots() {
    let tree = CowBtree::new();

    // Insert initial data
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();

    // Take snapshot before modification
    let snapshot = tree.snapshot();

    // Modify data - creates old pages
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(10)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 2), vec![Value::Int4(20)])
        .unwrap();

    let stats_before = tree.stats();
    assert!(
        stats_before.pending_gc_pages > 0,
        "should have pending GC pages"
    );
    assert_eq!(
        stats_before.active_snapshots, 1,
        "should have one active snapshot"
    );

    // Run vacuum - should NOT free pages referenced by snapshot
    let result = tree.vacuum();
    assert_eq!(
        result.pages_freed, 0,
        "should not free pages with active snapshot"
    );
    assert!(result.pages_pending > 0, "pages should still be pending");
    assert_eq!(result.min_active_version, Some(snapshot.version));

    // Snapshot still sees old data
    let snap_results = tree.scan_table_at_root(snapshot.root, 1).unwrap();
    assert_eq!(snap_results.len(), 1);
    assert_eq!(snap_results[0].1, vec![Value::Int4(1)]);

    // Release snapshot
    tree.release_snapshot(&snapshot);

    // Now vacuum should work
    let result2 = tree.vacuum();
    assert!(
        result2.pages_freed > 0,
        "should free pages after snapshot release"
    );
    assert_eq!(result2.pages_pending, 0, "no pages should be pending");
}

#[test]
fn test_stats_include_vacuum_info() {
    let tree = CowBtree::new();
    let stats = tree.stats();

    // New tree should have clean stats
    assert_eq!(stats.pending_gc_batches, 0);
    assert_eq!(stats.pending_gc_pages, 0);
    assert_eq!(stats.free_list_size, 0);
    assert_eq!(stats.active_snapshots, 0);

    // Insert and update to create pending pages
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(2)])
        .unwrap();

    let stats2 = tree.stats();
    assert!(
        stats2.pending_gc_pages > 0,
        "should have pending pages after update"
    );
    assert!(stats2.pending_gc_batches > 0, "should have pending batches");

    // Vacuum
    tree.vacuum();

    let stats3 = tree.stats();
    assert_eq!(stats3.pending_gc_pages, 0, "no pending after vacuum");
    assert!(stats3.free_list_size > 0, "pages in free list");
}

#[test]
fn test_auto_vacuum_threshold() {
    // Create tree with low threshold for testing
    let tree = CowBtree::with_vacuum_threshold(2);
    assert_eq!(tree.vacuum_threshold(), 2);

    // Insert data
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();

    // Take snapshot
    let snapshot = tree.snapshot();

    // Updates create pending pages
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(10)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(100)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1000)])
        .unwrap();

    let stats_before = tree.stats();
    // Pages are pending but can't be freed due to snapshot
    assert!(
        stats_before.pending_gc_pages >= 2,
        "should exceed threshold"
    );

    // Release snapshot - with threshold of 2, auto-vacuum should trigger
    tree.release_snapshot(&snapshot);

    // After release, vacuum should have been triggered
    // (pages should be freed since no more snapshots)
    let stats_after = tree.stats();
    assert_eq!(
        stats_after.pending_gc_pages, 0,
        "auto-vacuum should have run"
    );
    assert!(stats_after.free_list_size > 0, "pages in free list");
}

#[test]
fn test_page_reuse_from_free_list() {
    let tree = CowBtree::new();

    // Insert and update to create garbage
    for i in 0..5 {
        tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(i)])
            .unwrap();
    }

    // Vacuum to populate free list
    tree.vacuum();

    let stats_before = tree.stats();
    let free_before = stats_before.free_list_size;
    assert!(free_before > 0, "should have free pages");

    // New inserts should reuse pages from free list
    for i in 0..3 {
        tree.insert(BtreeKey::new(1, i + 100), vec![Value::Int4(i as i32)])
            .unwrap();
    }

    // Free list should have been used (note: may grow again due to CoW)
    // The key verification is that the tree works correctly
    let results = tree.scan_table(1).unwrap();
    assert_eq!(results.len(), 4); // original row 1 + 3 new rows
}

#[test]
fn test_vacuum_result() {
    let tree = CowBtree::new();

    // Empty tree vacuum
    let result = tree.vacuum();
    assert_eq!(result.pages_freed, 0);
    assert_eq!(result.pages_pending, 0);
    assert!(result.min_active_version.is_none());

    // Add data and create garbage
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(2)])
        .unwrap();

    // Vacuum and check result
    let result2 = tree.vacuum();
    assert!(result2.pages_freed > 0);
    assert_eq!(result2.pages_pending, 0);
}

#[test]
fn test_release_snapshot_and_vacuum() {
    let tree = CowBtree::new();

    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();

    let snapshot = tree.snapshot();

    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(10)])
        .unwrap();

    // Use the combined release + vacuum method
    let result = tree.release_snapshot_and_vacuum(&snapshot);
    assert!(result.pages_freed > 0, "should free pages");
    assert_eq!(result.pages_pending, 0);
    assert!(
        result.min_active_version.is_none(),
        "no more active snapshots"
    );
}

// ========================================================================
// BatchChanges Tests
// ========================================================================

#[test]
fn test_batch_changes_empty() {
    let batch = BatchChanges::new();
    assert!(batch.is_empty());
    assert_eq!(batch.len(), 0);
}

#[test]
fn test_batch_changes_insert() {
    let mut batch = BatchChanges::new();
    batch.insert(BtreeKey::new(1, 1), vec![Value::Int4(42)]);

    assert!(!batch.is_empty());
    assert_eq!(batch.len(), 1);
    assert_eq!(batch.inserts.len(), 1);
    assert_eq!(batch.deletes.len(), 0);
}

#[test]
fn test_batch_changes_delete() {
    let mut batch = BatchChanges::new();
    batch.delete(BtreeKey::new(1, 1));

    assert!(!batch.is_empty());
    assert_eq!(batch.len(), 1);
    assert_eq!(batch.inserts.len(), 0);
    assert_eq!(batch.deletes.len(), 1);
}

#[test]
fn test_batch_changes_mixed() {
    let mut batch = BatchChanges::new();
    batch.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)]);
    batch.insert(BtreeKey::new(1, 2), vec![Value::Int4(2)]);
    batch.delete(BtreeKey::new(1, 3));

    assert_eq!(batch.len(), 3);
    assert_eq!(batch.inserts.len(), 2);
    assert_eq!(batch.deletes.len(), 1);
}

// ========================================================================
// apply_batch Tests
// ========================================================================

#[test]
fn test_apply_batch_empty() {
    let tree = CowBtree::new();
    let batch = BatchChanges::new();

    let count = tree.apply_batch(batch).unwrap();
    assert_eq!(count, 0);
    assert!(tree.is_empty());
}

#[test]
fn test_apply_batch_single_insert() {
    let tree = CowBtree::new();
    let mut batch = BatchChanges::new();
    batch.insert(BtreeKey::new(1, 1), vec![Value::Int4(42)]);

    let count = tree.apply_batch(batch).unwrap();
    assert_eq!(count, 1);
    assert_eq!(tree.current_version(), 1);

    let result = tree.get(&BtreeKey::new(1, 1)).unwrap();
    assert_eq!(result, Some(vec![Value::Int4(42)]));
}

#[test]
fn test_apply_batch_multiple_inserts() {
    let tree = CowBtree::new();
    let mut batch = BatchChanges::new();
    for i in 0..10 {
        batch.insert(BtreeKey::new(1, i), vec![Value::Int4(i as i32)]);
    }

    let count = tree.apply_batch(batch).unwrap();
    assert_eq!(count, 10);
    assert_eq!(tree.current_version(), 1); // Single version for entire batch

    // Verify all entries
    for i in 0..10 {
        let result = tree.get(&BtreeKey::new(1, i)).unwrap();
        assert_eq!(result, Some(vec![Value::Int4(i as i32)]));
    }
}

#[test]
fn test_apply_batch_insert_and_delete() {
    let tree = CowBtree::new();

    // First batch: insert some data
    let mut batch1 = BatchChanges::new();
    batch1.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)]);
    batch1.insert(BtreeKey::new(1, 2), vec![Value::Int4(2)]);
    batch1.insert(BtreeKey::new(1, 3), vec![Value::Int4(3)]);
    tree.apply_batch(batch1).unwrap();

    // Second batch: delete one, insert another
    let mut batch2 = BatchChanges::new();
    batch2.delete(BtreeKey::new(1, 2));
    batch2.insert(BtreeKey::new(1, 4), vec![Value::Int4(4)]);

    let count = tree.apply_batch(batch2).unwrap();
    assert_eq!(count, 2);
    assert_eq!(tree.current_version(), 2); // Second batch increments version once

    // Verify state
    assert_eq!(
        tree.get(&BtreeKey::new(1, 1)).unwrap(),
        Some(vec![Value::Int4(1)])
    );
    assert_eq!(tree.get(&BtreeKey::new(1, 2)).unwrap(), None); // Deleted
    assert_eq!(
        tree.get(&BtreeKey::new(1, 3)).unwrap(),
        Some(vec![Value::Int4(3)])
    );
    assert_eq!(
        tree.get(&BtreeKey::new(1, 4)).unwrap(),
        Some(vec![Value::Int4(4)])
    );
}

#[test]
fn test_apply_batch_update() {
    let tree = CowBtree::new();

    // Insert initial data
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();

    // Batch update (insert with same key overwrites)
    let mut batch = BatchChanges::new();
    batch.insert(BtreeKey::new(1, 1), vec![Value::Int4(100)]);

    tree.apply_batch(batch).unwrap();

    let result = tree.get(&BtreeKey::new(1, 1)).unwrap();
    assert_eq!(result, Some(vec![Value::Int4(100)]));
}

#[test]
fn test_apply_batch_many_causes_splits() {
    let tree = CowBtree::new();
    let mut batch = BatchChanges::new();

    // Insert enough entries to cause splits
    for i in 0..100 {
        batch.insert(BtreeKey::new(1, i), vec![Value::Int4(i as i32)]);
    }

    let count = tree.apply_batch(batch).unwrap();
    assert_eq!(count, 100);

    // Verify all entries accessible
    for i in 0..100 {
        let result = tree.get(&BtreeKey::new(1, i)).unwrap();
        assert_eq!(result, Some(vec![Value::Int4(i as i32)]));
    }
}

#[test]
fn test_apply_batch_atomicity() {
    let tree = CowBtree::new();

    // Insert initial data
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();
    let initial_version = tree.current_version();

    // Take snapshot before batch
    let snapshot = tree.snapshot();

    // Apply batch
    let mut batch = BatchChanges::new();
    batch.insert(BtreeKey::new(1, 2), vec![Value::Int4(2)]);
    batch.insert(BtreeKey::new(1, 3), vec![Value::Int4(3)]);
    tree.apply_batch(batch).unwrap();

    // Snapshot should still see old state
    let snap_results = tree.scan_table_at_root(snapshot.root, 1).unwrap();
    assert_eq!(snap_results.len(), 1);

    // Current state should see all data
    let current_results = tree.scan_table(1).unwrap();
    assert_eq!(current_results.len(), 3);

    // Version should have incremented only once
    assert_eq!(tree.current_version(), initial_version + 1);

    tree.release_snapshot(&snapshot);
}

// ========================================================================
// Seqlock consistency: snapshot() never observes a torn (root, version)
// ========================================================================

/// A writer commits one row per `insert`, bumping BOTH root and version each
/// commit; several readers hammer `snapshot()` and verify that every observed
/// `Snapshot { version, root }` is a pair the writer actually published —
/// never torn (old root + new version, or vice versa).
///
/// Invariant (read-through, shape (a)): the writer inserts keys `0..N` into
/// table 1, so after the commit that produced version `v` the tree holds
/// exactly `v` rows (version 0 = empty = 0 rows). A reader scanning AT
/// `snapshot.root` must therefore see exactly `snapshot.version` rows. A torn
/// `(new version, old root)` (or the reverse) would make `rows(root)` disagree
/// with `version`, failing the assertion.
///
/// This is a stress/invariant test, not a deterministic race reproducer — the
/// production tear window is the two adjacent atomic stores inside
/// `publish_root`, which a seqlock makes unobservable. Many iterations give
/// the (now-closed) race a chance to surface a regression.
#[test]
fn test_snapshot_pair_never_torn_under_concurrent_writes() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::thread;

    const INSERTS: u64 = 2000;
    const READERS: usize = 4;

    let tree = Arc::new(CowBtree::new());
    let done = Arc::new(AtomicBool::new(false));

    // Reader threads: spin on snapshot() and assert root/version consistency.
    let mut readers = Vec::new();
    for _ in 0..READERS {
        let tree = Arc::clone(&tree);
        let done = Arc::clone(&done);
        readers.push(thread::spawn(move || {
            let mut observed: u64 = 0;
            while !done.load(Ordering::Relaxed) {
                let snapshot = tree.snapshot();
                // Read AT the snapshot's root; the snapshot's refcount pins
                // this version's pages, so the scan is safe against vacuum.
                let rows = tree
                    .scan_table_at_root(snapshot.root, 1)
                    .expect("scan at snapshot root");
                assert_eq!(
                    rows.len() as u64,
                    snapshot.version,
                    "torn snapshot: version {} but {} rows at root {}",
                    snapshot.version,
                    rows.len(),
                    snapshot.root,
                );
                tree.release_snapshot(&snapshot);
                observed += 1;
            }
            observed
        }));
    }

    // Writer thread: one row per commit, so each commit changes BOTH root and
    // version — exactly the publish the seqlock must make atomic.
    let writer = {
        let tree = Arc::clone(&tree);
        thread::spawn(move || {
            for i in 0..INSERTS {
                tree.insert(BtreeKey::new(1, i), vec![Value::Int8(i as i64)])
                    .expect("insert");
            }
        })
    };

    writer.join().expect("writer join");
    done.store(true, Ordering::Relaxed);

    let total_observed: u64 = readers
        .into_iter()
        .map(|h| h.join().expect("reader join"))
        .sum();

    // Final state is fully published.
    assert_eq!(tree.current_version(), INSERTS);
    let final_rows = tree.scan_table(1).expect("final scan");
    assert_eq!(final_rows.len() as u64, INSERTS);
    // Readers actually ran (guards against the loop never executing).
    assert!(total_observed > 0, "readers should observe snapshots");
}

/// Deterministic refcount accounting for the atomic `active_snapshots`: two
/// snapshots at the same version, then releases, must move `min_active_version`
/// exactly — the entry survives while any reference remains and is removed only
/// when it drains, and a later snapshot of the same version re-inserts it.
/// (`vacuum()` exposes `min_active_version`, so we observe it without reaching
/// into the private map.)
#[test]
fn test_snapshot_refcount_tracks_overlapping_readers() {
    let tree = CowBtree::new();
    for i in 0..3 {
        tree.insert(BtreeKey::new(1, i), vec![Value::Int8(i as i64)])
            .expect("insert");
    }

    let s1 = tree.snapshot();
    let version = s1.version;
    let s2 = tree.snapshot();
    assert_eq!(s2.version, version, "no commit between → same version");

    // Two references at `version` → it is the minimum active version.
    assert_eq!(tree.vacuum().min_active_version, Some(version));
    // One reference left (count 2 → 1): the entry must NOT be removed.
    tree.release_snapshot(&s1);
    assert_eq!(tree.vacuum().min_active_version, Some(version));
    // Last reference dropped (count 1 → 0): the entry is removed, nothing active.
    tree.release_snapshot(&s2);
    assert_eq!(tree.vacuum().min_active_version, None);
    // Re-snapshotting a drained version re-inserts the counter cleanly.
    let s3 = tree.snapshot();
    assert_eq!(tree.vacuum().min_active_version, Some(s3.version));
    tree.release_snapshot(&s3);
    assert_eq!(tree.vacuum().min_active_version, None);
}

/// Hammer the same-version hot path from many threads (the read-heavy steady
/// state: a constant version, so every `snapshot`/`release` bumps the *same*
/// counter and the count repeatedly hits zero and revives). A mistake in the
/// remove-if-still-zero re-check would lose increments (underflow/panic) or leak
/// zombie entries; the post-drain assertions catch both, and every get must see
/// the one committed value (never a freed/torn page).
#[test]
fn test_concurrent_same_version_churn_drains_clean() {
    use std::sync::Arc;
    use std::thread;

    const THREADS: usize = 8;
    const ITERS: usize = 50_000;

    let tree = Arc::new(CowBtree::new());
    tree.insert(BtreeKey::new(1, 0), vec![Value::Int8(7)])
        .expect("seed insert");

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let tree = Arc::clone(&tree);
            thread::spawn(move || {
                for _ in 0..ITERS {
                    let snapshot = tree.snapshot();
                    let got = tree
                        .get_at_root(snapshot.root, &BtreeKey::new(1, 0))
                        .expect("get at snapshot root");
                    assert_eq!(got, Some(vec![Value::Int8(7)]), "stale/torn read");
                    tree.release_snapshot(&snapshot);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("reader join");
    }

    // Every increment matched a decrement and every drained version was removed:
    // no active snapshot, no leaked entry, no underflow.
    assert_eq!(
        tree.vacuum().min_active_version,
        None,
        "refcount must drain to empty"
    );
    assert_eq!(
        tree.stats().active_snapshots,
        0,
        "no leaked snapshot entries"
    );
}

#[test]
fn test_fork_is_isolated() {
    let tree = CowBtree::new();
    for i in 0..5 {
        tree.insert(BtreeKey::new(1, i), vec![Value::Int4(i as i32)])
            .unwrap();
    }

    // Fork the in-memory tree at its committed state.
    let fork = tree.fork().expect("in-memory store forks");
    assert_eq!(fork.current_version(), tree.current_version());
    for i in 0..5 {
        assert_eq!(
            fork.get(&BtreeKey::new(1, i)).unwrap(),
            Some(vec![Value::Int4(i as i32)]),
            "fork starts identical to the base"
        );
    }

    // Write to the fork: a new key and an overwrite of an existing one.
    fork.insert(BtreeKey::new(1, 99), vec![Value::Int4(99)])
        .unwrap();
    fork.insert(BtreeKey::new(1, 0), vec![Value::Int4(1000)])
        .unwrap();

    // The base does not see the fork's writes (Arc make_mut copied the touched
    // leaf; the base's pages are untouched).
    assert_eq!(
        tree.get(&BtreeKey::new(1, 99)).unwrap(),
        None,
        "the fork's new key must not appear in the base"
    );
    assert_eq!(
        tree.get(&BtreeKey::new(1, 0)).unwrap(),
        Some(vec![Value::Int4(0)]),
        "the fork's overwrite must not mutate the base"
    );

    // Write to the base after forking: the fork does not see it either.
    tree.insert(BtreeKey::new(1, 50), vec![Value::Int4(50)])
        .unwrap();
    assert_eq!(
        fork.get(&BtreeKey::new(1, 50)).unwrap(),
        None,
        "a base write after the fork must not appear in the fork"
    );
    // The fork keeps its own mutations.
    assert_eq!(
        fork.get(&BtreeKey::new(1, 0)).unwrap(),
        Some(vec![Value::Int4(1000)])
    );
}

/// A seedable xorshift for the randomized audits below (no `rand` dependency).
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x % n
    }
}

/// The leaves reachable from the tree's current root.
fn leaf_count(tree: &CowBtree) -> usize {
    let mut stack = vec![tree.current_root()];
    let mut leaves = 0;
    while let Some(id) = stack.pop() {
        match tree.store().get(id).unwrap().expect("reachable page") {
            Page::Internal {
                first_child,
                entries,
                ..
            } => {
                stack.push(first_child);
                stack.extend(entries.iter().map(|e| e.child));
            }
            Page::Leaf { .. } => leaves += 1,
            Page::Free { .. } | Page::Overflow { .. } => {}
        }
    }
    leaves
}

/// Every page reachable from the tree's current root, each checked to fit one page.
fn assert_every_reachable_page_fits(tree: &CowBtree) -> usize {
    let mut stack = vec![tree.current_root()];
    let mut seen = 0;
    while let Some(id) = stack.pop() {
        if id == NULL_PAGE_ID {
            continue;
        }
        let page = tree.store().get(id).unwrap().expect("reachable page");
        assert!(
            page.check_fits().is_ok(),
            "page {id} is {} bytes, over one page",
            page.encoded_len()
        );
        match &page {
            Page::Internal {
                first_child,
                entries,
                ..
            } => {
                stack.push(*first_child);
                stack.extend(entries.iter().map(|e| e.child));
            }
            // An out-of-line value's chain is reachable through its entry.
            Page::Leaf { entries, .. } => {
                for entry in entries.iter() {
                    if let LeafValue::Overflow(r) = entry.value {
                        stack.push(r.head);
                    }
                }
            }
            Page::Overflow { header, .. } => stack.push(header.next_or_reserved),
            Page::Free { .. } => {}
        }
        seen += 1;
    }
    seen
}

/// `len` bytes (at least eight) of `fill` behind a prefix naming `row_id`, so no two
/// rows' values are equal and none carries a NUL.
fn unique_bytes(row_id: u64, fill: u8, len: usize) -> Vec<u8> {
    let mut bytes = format!("{row_id:07}.").into_bytes();
    bytes.resize(len.max(bytes.len()), fill);
    bytes
}

/// A data row of `len` bytes beside an index entry keyed on the same bytes, as the
/// server batches an indexed `INSERT`, with a quarter of the rows later rewritten to
/// another length and their index entry moved — random trees whose entries each fit
/// a page alone, at several maximum lengths. Every tree's every reachable page must
/// fit, and every row and index entry must read back.
#[test]
fn random_row_and_index_workloads_build_only_pages_that_fit() {
    // The longest value whose index key still fits an internal page as a separator:
    // `BtreeKey::index` adds 1 + 4 + 2 + 8 bytes around it.
    let max_index_value = MAX_KEY_LEN - 15;
    for (seed, max_len) in (1u64..).zip([200usize, 1000, 2000, 2700, max_index_value]) {
        for tree_no in 0..24u64 {
            let mut rng = Rng((seed << 32) ^ (tree_no + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let tree = CowBtree::new();
            let mut model: Vec<Vec<u8>> = Vec::new();
            for step in 0..60u64 {
                let len = 1 + rng.below(max_len as u64) as usize;
                let row_id = model.len() as u64;
                let bytes = unique_bytes(row_id, b'a' + (step % 26) as u8, len);
                let mut batch = BatchChanges::new();
                // A data row's value is bounded by the page, not by `max_len`.
                let row_value = String::from_utf8(bytes[..bytes.len().min(3900)].to_vec()).unwrap();
                batch.insert(BtreeKey::new(1, row_id), vec![Value::Text(row_value)]);
                batch.insert(BtreeKey::index(2, &bytes, row_id), Vec::new());
                tree.apply_batch(batch).unwrap_or_else(|e| {
                    panic!("seed {seed} tree {tree_no} step {step} insert: {e}")
                });
                model.push(bytes);
                if model.len() > 1 && rng.below(4) == 0 {
                    let k = rng.below(model.len() as u64 - 1) as usize;
                    let new_len = 1 + rng.below(max_len as u64) as usize;
                    let new_bytes = unique_bytes(k as u64, b'z', new_len);
                    let mut batch = BatchChanges::new();
                    batch.delete(BtreeKey::index(2, &model[k], k as u64));
                    let row_value =
                        String::from_utf8(new_bytes[..new_bytes.len().min(3900)].to_vec()).unwrap();
                    batch.insert(BtreeKey::new(1, k as u64), vec![Value::Text(row_value)]);
                    batch.insert(BtreeKey::index(2, &new_bytes, k as u64), Vec::new());
                    tree.apply_batch(batch).unwrap_or_else(|e| {
                        panic!("seed {seed} tree {tree_no} step {step} update: {e}")
                    });
                    model[k] = new_bytes;
                }
            }
            assert_every_reachable_page_fits(&tree);
            for (row_id, bytes) in model.iter().enumerate() {
                let row_id = row_id as u64;
                let want = String::from_utf8(bytes[..bytes.len().min(3900)].to_vec()).unwrap();
                assert_eq!(
                    tree.get(&BtreeKey::new(1, row_id)).unwrap(),
                    Some(vec![Value::Text(want)]),
                    "seed {seed} tree {tree_no}: row {row_id}"
                );
                assert_eq!(
                    tree.index_scan_at_root(tree.current_root(), 2, bytes)
                        .unwrap(),
                    vec![row_id],
                    "seed {seed} tree {tree_no}: index entry of row {row_id}"
                );
            }
        }
    }
}

/// Three entries that each fit a page alone, no two-way cut of which fits: 2 000 and
/// 2 000 bytes, then 3 900 between them. The split must cut three ways.
#[test]
fn entries_no_two_way_cut_fits_split_three_ways() {
    let tree = CowBtree::new();
    for (value, row_id) in [(b'a', 1u64), (b'c', 2)] {
        tree.insert(BtreeKey::index(1, &vec![value; 2000], row_id), Vec::new())
            .unwrap();
    }
    tree.insert(BtreeKey::index(1, &vec![b'b'; 3900], 3), Vec::new())
        .unwrap();
    assert_every_reachable_page_fits(&tree);
    assert_eq!(leaf_count(&tree), 3, "one leaf per entry: a three-way cut");
    for (value, len, row_id) in [(b'a', 2000, 1u64), (b'b', 3900, 3), (b'c', 2000, 2)] {
        assert_eq!(
            tree.index_scan_at_root(tree.current_root(), 1, &vec![value; len])
                .unwrap(),
            vec![row_id]
        );
    }
}

/// Separators long enough that an internal page holds only one of them force the
/// internal levels to split k ways and the root to grow more than one level in a
/// single batch — every page must still fit and every key read back.
#[test]
fn long_separators_split_internal_pages_and_grow_the_root() {
    let tree = CowBtree::new();
    let mut batch = BatchChanges::new();
    let keys: Vec<Vec<u8>> = (0..40u8)
        .map(|i| {
            let mut v = vec![b'k'; 3000];
            v[0] = b'a' + i % 26;
            v[1] = b'a' + i / 26;
            v
        })
        .collect();
    for (row_id, key) in keys.iter().enumerate() {
        batch.insert(BtreeKey::index(1, key, row_id as u64), Vec::new());
    }
    tree.apply_batch(batch).unwrap();
    assert_every_reachable_page_fits(&tree);
    // One separator per internal page: forty leaves need more than two internal levels.
    let mut height = 1;
    let mut id = tree.current_root();
    while let Some(Page::Internal { first_child, .. }) = tree.store().get(id).unwrap() {
        height += 1;
        id = first_child;
    }
    assert!(
        height >= 4,
        "the root must grow several levels, got height {height}"
    );
    for (row_id, key) in keys.iter().enumerate() {
        assert_eq!(
            tree.index_scan_at_root(tree.current_root(), 1, key)
                .unwrap(),
            vec![row_id as u64]
        );
    }
}

/// The in-memory page store refuses a page that would not fit a page on disk, as the
/// file store does, rather than holding it.
#[test]
fn the_in_memory_store_refuses_an_overfull_page() {
    let store = CowPageStore::new();
    let mut page = Page::new_leaf(store.allocate());
    if let Page::Leaf { entries, .. } = &mut page {
        let entries = Arc::make_mut(entries);
        for row_id in 0..2 {
            entries.push(LeafEntry {
                key: BtreeKey::new(1, row_id),
                value: LeafValue::Inline(vec![Value::Text("x".repeat(3000))]),
            });
        }
    }
    assert!(matches!(
        store.put(page.clone()),
        Err(PageStoreError::Overfull { .. })
    ));
    assert_eq!(store.page_count(), 0, "the refused page must not be held");
    // A commit's pages are refused whole: the page that fits, stored ahead of the
    // one that does not, is not held either.
    let fits = Page::new_leaf(store.allocate());
    assert!(matches!(
        store.put_batch(vec![fits, page]),
        Err(PageStoreError::Overfull { .. })
    ));
    assert_eq!(
        store.page_count(),
        0,
        "a refused batch must hold none of its pages"
    );
}

/// A key longer than an internal page can carry as a separator is refused before the
/// tree is touched: no version, no page.
#[test]
fn a_key_longer_than_a_separator_can_be_is_refused_up_front() {
    let tree = CowBtree::new();
    tree.insert(BtreeKey::new(1, 1), vec![Value::Int4(1)])
        .unwrap();
    let version = tree.current_version();
    // 1 + 4 + (4 030 + 2) + 8 = 4 045 bytes, one over the bound.
    let key = BtreeKey::index(1, &vec![b'x'; MAX_KEY_LEN - 14], 1);
    assert_eq!(key.encoded_len(), MAX_KEY_LEN + 1);
    assert_eq!(
        tree.insert(key, Vec::new()),
        Err(CowBtreeError::KeyTooLarge {
            len: MAX_KEY_LEN + 1
        })
    );
    assert_eq!(tree.current_version(), version);
    // One byte shorter is stored.
    tree.insert(
        BtreeKey::index(1, &vec![b'x'; MAX_KEY_LEN - 15], 1),
        Vec::new(),
    )
    .unwrap();
    assert_every_reachable_page_fits(&tree);
}

// ============================================================================
// Overflow chains: a leaf value too long to keep inline
// ============================================================================

/// A row value of `len` bytes of `fill` — a `Text` whose encoding is `len` plus a
/// few bytes of framing.
fn big_row(fill: char, len: usize) -> Vec<Value> {
    vec![
        Value::Int4(len as i32),
        Value::Text(fill.to_string().repeat(len)),
    ]
}

/// The leaf value `key` is stored under, as its page holds it.
fn stored_value(tree: &CowBtree, key: &BtreeKey) -> LeafValue {
    let leaf_id = tree.find_leaf_from(tree.current_root(), key).unwrap();
    let Some(Page::Leaf { entries, .. }) = tree.store().get(leaf_id).unwrap() else {
        panic!("leaf");
    };
    let idx = entries.binary_search_by(|e| e.key.cmp(key)).unwrap();
    entries[idx].value.clone()
}

/// Pages allocated and not on the free list: what the tree holds, garbage pending
/// collection included.
fn live_pages(tree: &CowBtree) -> u64 {
    tree.store().next_page_id() - tree.store().free_list_size() as u64
}

/// A value over the inline limit goes out of line into a chain of as many pages as
/// its encoding needs, and reads back identical, point and range; one under stays
/// inline.
#[test]
fn a_long_value_is_stored_out_of_line_and_reads_back() {
    let tree = CowBtree::new();
    let short = big_row('s', INLINE_VALUE_MAX - 20);
    let long = big_row('l', 3 * DATA_SIZE + 17);
    tree.insert(BtreeKey::new(1, 1), short.clone()).unwrap();
    tree.insert(BtreeKey::new(1, 2), long.clone()).unwrap();

    assert!(matches!(
        stored_value(&tree, &BtreeKey::new(1, 1)),
        LeafValue::Inline(_)
    ));
    let LeafValue::Overflow(r) = stored_value(&tree, &BtreeKey::new(1, 2)) else {
        panic!("a value over the inline limit must be stored out of line");
    };
    assert_eq!(r.page_count(), 4, "3 full links and a remainder");
    assert_eq!(tree.get(&BtreeKey::new(1, 2)).unwrap(), Some(long.clone()));
    assert_eq!(
        tree.scan_table(1).unwrap(),
        vec![(BtreeKey::new(1, 1), short), (BtreeKey::new(1, 2), long)]
    );
    assert_every_reachable_page_fits(&tree);
}

/// A replaced or deleted out-of-line value's chain goes back to the free list once
/// no snapshot can read it, so a churn of long values returns the store to where it
/// started; and a snapshot taken before the churn keeps reading the old value, its
/// chain held until the snapshot is released.
#[test]
fn a_chain_is_reclaimed_when_nothing_can_read_it() {
    let tree = CowBtree::new();
    tree.insert(BtreeKey::new(1, 1), big_row('a', 100)).unwrap();
    let old = big_row('b', 2 * DATA_SIZE + 5);
    tree.insert(BtreeKey::new(1, 2), old.clone()).unwrap();
    tree.vacuum();
    let baseline = live_pages(&tree);

    let snap = tree.snapshot();
    let new = big_row('c', 5 * DATA_SIZE);
    tree.insert(BtreeKey::new(1, 2), new.clone()).unwrap();
    tree.vacuum();
    // Allocate after the vacuum, so a page freed too early would be overwritten now.
    tree.insert(BtreeKey::new(1, 3), big_row('q', 6 * DATA_SIZE))
        .unwrap();
    tree.delete(&BtreeKey::new(1, 3)).unwrap();
    assert_eq!(tree.get(&BtreeKey::new(1, 2)).unwrap(), Some(new));
    assert_eq!(
        tree.get_at_root(snap.root, &BtreeKey::new(1, 2)).unwrap(),
        Some(old),
        "a snapshot from before the replace reads the old value through vacuum"
    );
    tree.release_snapshot_and_vacuum(&snap);

    // Replace back to a short value, then delete it: every chain has gone.
    tree.insert(BtreeKey::new(1, 2), big_row('d', 10)).unwrap();
    tree.delete(&BtreeKey::new(1, 2)).unwrap();
    tree.insert(BtreeKey::new(1, 2), big_row('e', 2 * DATA_SIZE + 5))
        .unwrap();
    tree.vacuum();
    assert_eq!(
        live_pages(&tree),
        baseline,
        "the replaced and deleted chains must all be reclaimed"
    );
    assert_every_reachable_page_fits(&tree);
}

/// A key inserted twice in one batch: the first value's chain was created by the
/// batch and is dropped by it, never written — no page is left behind for it.
#[test]
fn a_chain_replaced_within_its_own_batch_is_never_written() {
    let tree = CowBtree::new();
    tree.insert(BtreeKey::new(1, 0), vec![Value::Int4(0)])
        .unwrap();
    tree.vacuum();
    let before = live_pages(&tree);
    let mut batch = BatchChanges::new();
    batch.insert(BtreeKey::new(1, 1), big_row('x', 3 * DATA_SIZE));
    batch.insert(BtreeKey::new(1, 1), big_row('y', 2 * DATA_SIZE));
    tree.apply_batch(batch).unwrap();
    tree.vacuum();
    assert_eq!(
        tree.get(&BtreeKey::new(1, 1)).unwrap(),
        Some(big_row('y', 2 * DATA_SIZE))
    );
    // The one leaf copied (the old one freed) plus the surviving chain's 3 pages.
    let LeafValue::Overflow(r) = stored_value(&tree, &BtreeKey::new(1, 1)) else {
        panic!("out of line");
    };
    assert_eq!(live_pages(&tree), before + r.page_count());
}

/// A fork shares the source's chains, and a write on either side leaves the other's
/// value whole.
#[test]
fn a_fork_shares_chains_and_each_side_keeps_its_own() {
    let tree = CowBtree::new();
    let base = big_row('f', 4 * DATA_SIZE);
    tree.insert(BtreeKey::new(1, 1), base.clone()).unwrap();
    let fork = tree.fork().expect("in-memory store forks");
    fork.insert(BtreeKey::new(1, 1), big_row('g', DATA_SIZE * 2))
        .unwrap();
    fork.vacuum();
    tree.delete(&BtreeKey::new(1, 1)).unwrap();
    tree.vacuum();
    tree.insert(BtreeKey::new(1, 2), big_row('h', DATA_SIZE * 4))
        .unwrap();
    assert_eq!(
        fork.get(&BtreeKey::new(1, 1)).unwrap(),
        Some(big_row('g', DATA_SIZE * 2))
    );
    assert_eq!(tree.get(&BtreeKey::new(1, 1)).unwrap(), None);
    assert_eq!(
        tree.get(&BtreeKey::new(1, 2)).unwrap(),
        Some(big_row('h', DATA_SIZE * 4))
    );
    // A fork taken before the source's writes still reads the value it shared.
    let early = CowBtree::new();
    early.insert(BtreeKey::new(1, 1), base.clone()).unwrap();
    let early_fork = early.fork().unwrap();
    early.delete(&BtreeKey::new(1, 1)).unwrap();
    early.vacuum();
    early
        .insert(BtreeKey::new(1, 3), big_row('i', DATA_SIZE * 4))
        .unwrap();
    assert_eq!(early_fork.get(&BtreeKey::new(1, 1)).unwrap(), Some(base));
}

/// A page store that counts its reads, can be told to stop answering after a
/// ceiling, and can be told to refuse a commit's pages as overfull — the store double
/// the chain-walk and batch-refusal guards read through.
struct ProbeStore {
    inner: CowPageStore,
    reads: Arc<AtomicUsize>,
    /// Reads past this many are an I/O error, so a walk without bound ends loud
    /// (and small) under a mutation rather than exhausting memory.
    ceiling: Arc<AtomicUsize>,
    refuse_batches: Arc<std::sync::atomic::AtomicBool>,
    /// While set, a commit's page writes (`put_batch`) fail with a plain I/O error.
    fail_writes: Arc<std::sync::atomic::AtomicBool>,
    /// While set, the data `flush` fails with a plain I/O error.
    fail_flush: Arc<std::sync::atomic::AtomicBool>,
}

/// The handles a test keeps on a [`ProbeStore`] it has boxed into a tree.
struct Probe {
    reads: Arc<AtomicUsize>,
    ceiling: Arc<AtomicUsize>,
    refuse_batches: Arc<std::sync::atomic::AtomicBool>,
    fail_writes: Arc<std::sync::atomic::AtomicBool>,
    fail_flush: Arc<std::sync::atomic::AtomicBool>,
}

impl Probe {
    /// Reads since the last call.
    fn take_reads(&self) -> usize {
        self.reads.swap(0, Ordering::SeqCst)
    }
}

fn probe_tree() -> (CowBtree, Probe) {
    probe_tree_with_root(NULL_PAGE_ID, 0)
}

/// [`probe_tree`] over a tree that starts at `root` and `version`, for a caller that
/// writes the pages itself.
fn probe_tree_with_root(root: PageId, version: u64) -> (CowBtree, Probe) {
    let probe = Probe {
        reads: Arc::new(AtomicUsize::new(0)),
        ceiling: Arc::new(AtomicUsize::new(usize::MAX)),
        refuse_batches: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        fail_writes: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        fail_flush: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let store = ProbeStore {
        inner: CowPageStore::new(),
        reads: Arc::clone(&probe.reads),
        ceiling: Arc::clone(&probe.ceiling),
        refuse_batches: Arc::clone(&probe.refuse_batches),
        fail_writes: Arc::clone(&probe.fail_writes),
        fail_flush: Arc::clone(&probe.fail_flush),
    };
    let tree = CowBtree::with_store(Box::new(store), root, version, usize::MAX);
    (tree, probe)
}

impl PageStore for ProbeStore {
    fn allocate(&self) -> PageId {
        self.inner.allocate()
    }
    fn get(&self, page_id: PageId) -> PageStoreResult<Option<Page>> {
        if self.reads.fetch_add(1, Ordering::SeqCst) >= self.ceiling.load(Ordering::SeqCst) {
            return Err(PageStoreError::Io(
                "read ceiling: a walk read more pages than it had any reason to".into(),
            ));
        }
        self.inner.get(page_id)
    }
    fn put(&self, page: Page) -> PageStoreResult<()> {
        self.inner.put(page)
    }
    fn put_batch(&self, pages: Vec<Page>) -> PageStoreResult<()> {
        if self.refuse_batches.load(Ordering::SeqCst) {
            return Err(PageStoreError::Overfull {
                page_id: pages.first().map_or(0, Page::page_id),
                bytes: PAGE_SIZE + 1,
            });
        }
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(PageStoreError::Io("injected write failure".into()));
        }
        self.inner.put_batch(pages)
    }
    fn flush(&self) -> PageStoreResult<()> {
        if self.fail_flush.load(Ordering::SeqCst) {
            return Err(PageStoreError::Io("injected flush failure".into()));
        }
        Ok(())
    }
    fn free_pages(&self, page_ids: Vec<PageId>) {
        self.inner.free_pages(page_ids);
    }
    fn page_count(&self) -> usize {
        self.inner.page_count()
    }
    fn free_list_size(&self) -> usize {
        self.inner.free_list_size()
    }
    fn next_page_id(&self) -> PageId {
        self.inner.next_page_id()
    }
    fn free_list(&self) -> Vec<PageId> {
        self.inner.free_list()
    }
}

/// A stored chain: the key whose entry references it, the reference, and its links.
struct Chain {
    key: BtreeKey,
    r: OverflowRef,
    ids: Vec<PageId>,
}

impl Chain {
    fn of(tree: &CowBtree, key: BtreeKey) -> Self {
        let LeafValue::Overflow(r) = stored_value(tree, &key) else {
            panic!("out of line");
        };
        let mut ids = vec![r.head];
        while let Some(Page::Overflow { header, .. }) =
            tree.store().get(*ids.last().unwrap()).unwrap()
        {
            if header.next_or_reserved == NULL_PAGE_ID {
                break;
            }
            ids.push(header.next_or_reserved);
        }
        Self { key, r, ids }
    }
}

/// Rewrite a stored link's payload and next pointer in place.
fn relink(store: &dyn PageStore, id: PageId, data: Option<Vec<u8>>, next: PageId) {
    let Some(Page::Overflow { data: old, .. }) = store.get(id).unwrap() else {
        panic!("link {id}");
    };
    let data = data.unwrap_or_else(|| old.to_vec());
    store.put(Page::new_overflow(id, data, next)).unwrap();
}

/// Rewrite the reference `key`'s leaf entry holds, in place.
fn set_reference(tree: &CowBtree, key: &BtreeKey, r: OverflowRef) {
    let leaf_id = tree.find_leaf_from(tree.current_root(), key).unwrap();
    let Some(mut leaf) = tree.store().get(leaf_id).unwrap() else {
        panic!("leaf");
    };
    if let Page::Leaf { entries, .. } = &mut leaf {
        let entries = Arc::make_mut(entries);
        let idx = entries.binary_search_by(|e| e.key.cmp(key)).unwrap();
        entries[idx].value = LeafValue::Overflow(r);
    }
    tree.store().put(leaf).unwrap();
}

/// A way to damage a stored chain; returns the page the refusal must name.
type Damage = fn(&CowBtree, &Chain) -> PageId;

/// Each of these shapes of a damaged chain is `CorruptChain` naming where it stopped
/// agreeing with its reference — on a read, a range scan, and a release (a delete
/// batch with no read before it) — and a read or a release of it reads no more pages
/// than the descent and the chain's own links: a chain whose reference claims more
/// links than the store has ever
/// allocated is refused before a link is read, and one that loops is refused at the
/// first revisit, not after as many hops as its reference claims. A refused release
/// leaves the tree, the pages pending collection and the free list as they were: the
/// batch's error returns before its working set is stored, and the ids it had
/// superseded go with the working set.
#[test]
fn a_damaged_chain_is_a_typed_corruption_error() {
    let value = big_row('z', 2 * DATA_SIZE + 100);
    let damage: [(&str, Damage); 11] = [
        ("a link past the end of the store", |tree, c| {
            let past = tree.store().next_page_id() + 1_000;
            relink(tree.store(), c.ids[0], None, past);
            past
        }),
        ("a link at the top of the id space", |tree, c| {
            relink(tree.store(), c.ids[0], None, NULL_PAGE_ID - 1);
            NULL_PAGE_ID - 1
        }),
        ("a link of another kind", |tree, c| {
            tree.store().put(Page::new_leaf(c.ids[1])).unwrap();
            c.ids[1]
        }),
        ("a short link", |tree, c| {
            // Not the head: a short head and a failed decode would name the same page.
            let Some(Page::Overflow { data, .. }) = tree.store().get(c.ids[1]).unwrap() else {
                panic!()
            };
            relink(
                tree.store(),
                c.ids[1],
                Some(data[..DATA_SIZE - 1].to_vec()),
                c.ids[2],
            );
            c.ids[1]
        }),
        ("a chain that goes on past its last link", |tree, c| {
            relink(tree.store(), c.ids[2], None, c.ids[0]);
            c.ids[2]
        }),
        ("a chain that ends before its last link", |tree, c| {
            relink(tree.store(), c.ids[1], None, NULL_PAGE_ID);
            c.ids[1]
        }),
        ("a chain that loops back", |tree, c| {
            relink(tree.store(), c.ids[1], None, c.ids[0]);
            c.ids[0]
        }),
        ("a reference shorter than its chain", |tree, c| {
            set_reference(
                tree,
                &c.key,
                OverflowRef {
                    head: c.r.head,
                    len: DATA_SIZE as u64,
                },
            );
            c.ids[0]
        }),
        ("a reference longer than the store could hold", |tree, c| {
            let pages = tree.store().next_page_id() + 1;
            set_reference(
                tree,
                &c.key,
                OverflowRef {
                    head: c.r.head,
                    len: pages * DATA_SIZE as u64,
                },
            );
            c.ids[0]
        }),
        ("a one-page loop as long as the store", |tree, c| {
            let pages = tree.store().next_page_id();
            set_reference(
                tree,
                &c.key,
                OverflowRef {
                    head: c.r.head,
                    len: pages * DATA_SIZE as u64,
                },
            );
            relink(tree.store(), c.ids[0], None, c.ids[0]);
            c.ids[0]
        }),
        ("a one-page loop claiming a terabyte", |tree, c| {
            set_reference(
                tree,
                &c.key,
                OverflowRef {
                    head: c.r.head,
                    len: 1 << 40,
                },
            );
            relink(tree.store(), c.ids[0], None, c.ids[0]);
            c.ids[0]
        }),
    ];
    for (name, damage) in damage {
        let (tree, probe) = probe_tree();
        // Other chains, so the store has allocated far more pages than one chain has.
        for row_id in 2..120 {
            tree.insert(BtreeKey::new(1, row_id), big_row('f', 2 * DATA_SIZE))
                .unwrap();
        }
        tree.insert(BtreeKey::new(1, 1), value.clone()).unwrap();
        tree.insert(BtreeKey::new(1, 0), vec![Value::Int4(0)])
            .unwrap();
        tree.vacuum();
        // What a descent to the entry reads, measured on its inline neighbour.
        probe.take_reads();
        tree.get(&BtreeKey::new(1, 0)).unwrap();
        let descent = probe.take_reads();
        let chain = Chain::of(&tree, BtreeKey::new(1, 1));
        assert_eq!(chain.ids.len(), 3, "{name}");
        assert!(tree.store().next_page_id() > 300, "{name}");
        let expected = damage(&tree, &chain);
        let refused = |result: Result<()>| {
            matches!(
                result,
                Err(CowBtreeError::Storage(PageStoreError::CorruptChain { head, page }))
                    if head == chain.r.head && page == expected
            )
        };
        // Past a hundred thousand reads the store stops answering: a walk without
        // bound fails the assertions below with an I/O error rather than running on.
        probe.ceiling.store(100_000, Ordering::SeqCst);
        probe.take_reads();
        let got = tree.get(&chain.key).map(|_| ());
        let reads = probe.take_reads();
        assert!(refused(got.clone()), "{name}: get: {got:?}");
        // The descent and at most the chain's three links.
        assert!(reads <= descent + 3, "{name}: get read {reads} pages");
        let scanned = tree.scan_table(1).map(|_| ());
        assert!(refused(scanned.clone()), "{name}: scan: {scanned:?}");

        let version = tree.current_version();
        let pending = tree.pending_gc_count();
        let free = tree.store().free_list();
        let mut batch = BatchChanges::new();
        batch.delete(chain.key.clone());
        probe.take_reads();
        let released = tree.apply_batch(batch).map(|_| ());
        let reads = probe.take_reads();
        assert!(refused(released.clone()), "{name}: release: {released:?}");
        assert!(reads <= descent + 3, "{name}: release read {reads} pages");
        assert_eq!(tree.current_version(), version, "{name}");
        assert_eq!(tree.pending_gc_count(), pending, "{name}");
        tree.vacuum();
        assert_eq!(
            tree.store().free_list(),
            free,
            "{name}: the refused release freed a page"
        );
    }
}

/// Two entries naming one chain — a shape only a damaged store holds — read back the
/// same value, but releasing both in one batch is refused rather than superseding the
/// chain's pages twice, which would put each on the free list twice and hand it out
/// to two owners.
#[test]
fn a_chain_two_entries_name_is_not_released_twice() {
    let tree = CowBtree::new();
    let value = big_row('s', 2 * DATA_SIZE);
    tree.insert(BtreeKey::new(1, 1), value.clone()).unwrap();
    tree.insert(BtreeKey::new(1, 2), big_row('t', 10)).unwrap();
    let chain = Chain::of(&tree, BtreeKey::new(1, 1));
    set_reference(&tree, &BtreeKey::new(1, 2), chain.r);
    assert_eq!(tree.get(&BtreeKey::new(1, 2)).unwrap(), Some(value));

    let version = tree.current_version();
    let mut batch = BatchChanges::new();
    batch.delete(BtreeKey::new(1, 1));
    batch.delete(BtreeKey::new(1, 2));
    let result = tree.apply_batch(batch);
    assert!(
        matches!(
            result,
            Err(CowBtreeError::Storage(PageStoreError::CorruptChain { head, page }))
                if head == chain.r.head && page == chain.ids[0]
        ),
        "{result:?}"
    );
    assert_eq!(tree.current_version(), version);
    tree.vacuum();
    let free = tree.store().free_list();
    let distinct: HashSet<PageId> = free.iter().copied().collect();
    assert_eq!(distinct.len(), free.len(), "free list {free:?}");
}

/// A store's refusal of a commit's pages as overfull publishes nothing and hands the
/// batch's fresh page ids back, chain links included: the tree, its version and the
/// pages it holds are as before, and the next commit reuses the ids.
#[test]
fn a_batch_the_store_refuses_as_overfull_publishes_nothing_and_frees_its_pages() {
    let (tree, probe) = probe_tree();
    tree.insert(BtreeKey::new(1, 1), big_row('a', 100)).unwrap();
    tree.insert(BtreeKey::new(1, 2), big_row('b', 3 * DATA_SIZE))
        .unwrap();
    tree.vacuum();
    let version = tree.current_version();
    let root = tree.current_root();
    let live = live_pages(&tree);

    probe.refuse_batches.store(true, Ordering::SeqCst);
    let mut batch = BatchChanges::new();
    batch.insert(BtreeKey::new(1, 3), big_row('c', 4 * DATA_SIZE));
    batch.insert(BtreeKey::new(1, 4), big_row('d', 50));
    let result = tree.apply_batch(batch);
    assert!(
        matches!(
            result,
            Err(CowBtreeError::Storage(PageStoreError::Overfull { .. }))
        ),
        "{result:?}"
    );
    // What a statement meets: an execution error, `22000`.
    let refusal = pglet_core::Error::from(crate::StorageError::from(result.unwrap_err()));
    assert_eq!(refusal.sqlstate(), "22000", "{refusal}");
    assert_eq!(tree.current_version(), version);
    assert_eq!(tree.current_root(), root);
    assert_eq!(
        live_pages(&tree),
        live,
        "the refused batch's fresh pages must go back to the free list"
    );
    assert_eq!(tree.pending_gc_count(), (0, 0));

    probe.refuse_batches.store(false, Ordering::SeqCst);
    tree.insert(BtreeKey::new(1, 3), big_row('c', 4 * DATA_SIZE))
        .unwrap();
    assert_eq!(
        tree.get(&BtreeKey::new(1, 3)).unwrap(),
        Some(big_row('c', 4 * DATA_SIZE))
    );
    assert_eq!(
        tree.get(&BtreeKey::new(1, 2)).unwrap(),
        Some(big_row('b', 3 * DATA_SIZE))
    );
}

/// A batch whose page writes or data `flush` fail with an error the store survives
/// publishes nothing and hands its fresh page ids back, as the overfull refusal does:
/// the tree reads as before and holds the same pages, and the next commit succeeds.
/// (The file store answers a refused write `WriteRefused`, and the tree fails it unless the
/// batch is discardable — `CowBtree::settle_refusal` — which `double_meta.rs`'s and
/// `durability.rs`'s tests cover.)
#[test]
fn a_batch_whose_write_or_flush_fails_publishes_nothing_and_frees_its_pages() {
    for fail_flush in [false, true] {
        let (tree, probe) = probe_tree();
        tree.insert(BtreeKey::new(1, 1), big_row('a', 100)).unwrap();
        tree.insert(BtreeKey::new(1, 2), big_row('b', 3 * DATA_SIZE))
            .unwrap();
        tree.vacuum();
        let version = tree.current_version();
        let root = tree.current_root();
        let live = live_pages(&tree);

        let flag = if fail_flush {
            &probe.fail_flush
        } else {
            &probe.fail_writes
        };
        flag.store(true, Ordering::SeqCst);
        let mut batch = BatchChanges::new();
        batch.insert(BtreeKey::new(1, 3), big_row('c', 4 * DATA_SIZE));
        batch.insert(BtreeKey::new(1, 4), big_row('d', 50));
        let result = tree.apply_batch(batch);
        flag.store(false, Ordering::SeqCst);
        assert!(result.is_err(), "flush {fail_flush}: {result:?}");
        assert_eq!(tree.current_version(), version, "flush {fail_flush}");
        assert_eq!(tree.current_root(), root, "flush {fail_flush}");
        assert_eq!(
            live_pages(&tree),
            live,
            "flush {fail_flush}: the failed batch's fresh pages must go back to the free list"
        );
        tree.insert(BtreeKey::new(1, 3), big_row('c', 4 * DATA_SIZE))
            .unwrap();
        assert_eq!(
            tree.get(&BtreeKey::new(1, 3)).unwrap(),
            Some(big_row('c', 4 * DATA_SIZE))
        );
    }
}

/// An internal page over `children`: `first_child` the first, then one entry per
/// further child, keyed `(1, 10 * i)`.
fn internal_over(id: PageId, children: &[PageId]) -> Page {
    let entries = children[1..]
        .iter()
        .enumerate()
        .map(|(i, &child)| InternalEntry {
            key: BtreeKey::new(1, 10 * (i as u64 + 1)),
            child,
        })
        .collect::<Vec<_>>();
    let mut header = PageHeader::new(id, PageFlags::Internal);
    header.count = entries.len() as u16;
    Page::Internal {
        header,
        first_child: children[0],
        entries: Arc::new(entries),
    }
}

/// A leaf holding `(1, k)` for each `k`.
fn leaf_of(id: PageId, keys: &[u64]) -> Page {
    let mut page = Page::new_leaf(id);
    if let Page::Leaf { entries, header } = &mut page {
        let entries = Arc::make_mut(entries);
        for &k in keys {
            entries.push(LeafEntry {
                key: BtreeKey::new(1, k),
                value: LeafValue::Inline(vec![Value::Int8(k as i64)]),
            });
        }
        header.count = entries.len() as u16;
    }
    page
}

/// A tree over `pages`, rooted at page 0, in an in-memory store whose allocator has
/// passed every id `pages` uses.
///
/// The store is a [`ProbeStore`] whose reads stop at four times the pages it holds
/// (and 64 more): a walk this file's tests expect to be bounded, run without its
/// bound, then fails on `read ceiling` in bounded time and memory — a clean `FAILED`
/// naming the walk's assertion — rather than looping until killed or growing a path
/// past the memory cap, which stalls the whole test binary with no `FAILED` line.
fn crafted_tree(pages: Vec<Page>) -> CowBtree {
    let (tree, probe) = probe_tree_with_root(0, 1);
    let store = tree.store();
    let top = pages.iter().map(Page::page_id).max().unwrap_or(0);
    while store.allocate() < top {}
    probe.ceiling.store(pages.len() * 4 + 64, Ordering::SeqCst);
    for page in pages {
        store.put(page).unwrap();
    }
    tree
}

fn is_corrupt_tree(result: &Result<impl std::fmt::Debug>, page: PageId) -> bool {
    matches!(result, Err(CowBtreeError::Storage(PageStoreError::CorruptTree { page: p })) if *p == page)
}

/// A page that names an ancestor as its child is refused as a corrupt tree by every
/// walk — a point read, a range scan and a write's descent — naming the page reached
/// twice, where each used to follow it without end.
#[test]
fn a_tree_page_naming_an_ancestor_is_refused_by_every_walk() {
    // 0 → 1 → 0: the internal page 1's first child is the root.
    let tree = crafted_tree(vec![
        internal_over(0, &[1, 2]),
        internal_over(1, &[0, 2]),
        leaf_of(2, &[15]),
    ]);
    let key = BtreeKey::new(1, 5);
    assert!(is_corrupt_tree(&tree.get(&key), 0), "{:?}", tree.get(&key));
    let scan = tree.scan_table(1);
    assert!(is_corrupt_tree(&scan, 0), "{scan:?}");
    let write = tree.insert(key, vec![Value::Int8(5)]);
    assert!(is_corrupt_tree(&write, 0), "{write:?}");
    // The page itself: a root naming itself.
    let tree = crafted_tree(vec![internal_over(0, &[0, 1]), leaf_of(1, &[15])]);
    assert!(is_corrupt_tree(&tree.get(&BtreeKey::new(1, 1)), 0));
    assert!(is_corrupt_tree(&tree.scan_table(1), 0));
}

/// Two pages naming one child — a DAG, which no honest tree holds — is refused by a
/// scan rather than emitting the shared subtree twice; the control, the same shape
/// with two distinct leaves, reads every row once.
#[test]
fn a_child_two_pages_name_is_refused_by_a_scan() {
    let shared = crafted_tree(vec![internal_over(0, &[1, 1]), leaf_of(1, &[1, 2])]);
    let scan = shared.scan_table(1);
    assert!(is_corrupt_tree(&scan, 1), "{scan:?}");

    let honest = crafted_tree(vec![
        internal_over(0, &[1, 2]),
        leaf_of(1, &[1, 2]),
        leaf_of(2, &[11, 12]),
    ]);
    let keys: Vec<u64> = honest
        .scan_table(1)
        .unwrap()
        .iter()
        .map(|(k, _)| k.row_id())
        .collect();
    assert_eq!(keys, vec![1, 2, 11, 12]);
}

/// A chain of single-child internal pages deeper than any honest tree — the walks
/// run from their own stack, not the thread's, so a scan and a point read over it
/// answer rather than overflow the test thread's stack.
#[test]
fn a_deep_chain_of_pages_does_not_overflow_the_stack() {
    const DEPTH: u64 = 60_000;
    let mut pages: Vec<Page> = (0..DEPTH).map(|id| internal_over(id, &[id + 1])).collect();
    pages.push(leaf_of(DEPTH, &[7]));
    let tree = crafted_tree(pages);
    assert_eq!(tree.scan_table(1).unwrap().len(), 1);
    assert_eq!(
        tree.get(&BtreeKey::new(1, 7)).unwrap(),
        Some(vec![Value::Int8(7)])
    );
}

/// `last_key_in_range_at_root` answers the last key a range scan of the same band
/// collects, over a tree of several levels, for bands that start and end inside a leaf,
/// span leaves, hold nothing, and run past the last key.
#[test]
fn the_last_key_in_a_range_is_the_range_scans_last() {
    let tree = CowBtree::new();
    let mut batch = BatchChanges::new();
    for table in 1..=3_u32 {
        for row in 0..2_000_u64 {
            batch.insert(BtreeKey::new(table, row), vec![Value::Int4(0)]);
        }
    }
    for index in [4_u32, 9, 11] {
        for row in 0..1_500_u64 {
            batch.insert(BtreeKey::index(index, &row.to_be_bytes(), row), Vec::new());
        }
    }
    tree.apply_batch(batch).unwrap();
    let root = tree.current_root();
    assert!(tree.stats().page_count > 100, "a tree of several levels");
    let bands = [
        (BtreeKey::table_start(1), BtreeKey::table_end(1)),
        (BtreeKey::new(2, 17), BtreeKey::new(2, 1_234)),
        (BtreeKey::table_start(1), BtreeKey::table_end(3)),
        (BtreeKey::table_start(5), BtreeKey::table_end(9)),
        (
            BtreeKey::index_table_start(0),
            BtreeKey::sys_catalog_start(),
        ),
        (
            BtreeKey::index_table_start(0),
            BtreeKey::index_table_start(10),
        ),
        (
            BtreeKey::index_table_start(5),
            BtreeKey::index_table_start(9),
        ),
        (
            BtreeKey::index_table_start(12),
            BtreeKey::sys_catalog_start(),
        ),
        (BtreeKey::new(0, 0), BtreeKey::sys_catalog_start()),
    ];
    for (start, end) in bands {
        let scanned = tree
            .range_scan_at_root(root, &start, &end)
            .unwrap()
            .pop()
            .map(|(key, _)| key);
        assert_eq!(
            tree.last_key_in_range_at_root(root, &start, &end).unwrap(),
            scanned,
            "[{start:?}, {end:?})"
        );
    }
}
