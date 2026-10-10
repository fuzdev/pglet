#![no_main]
//! Coverage-guided durability fuzz over a (mutated) `DoubleMetaStore` file image,
//! in two legs per input.
//!
//! libFuzzer mutates `data` as the raw bytes of a double-meta file. Seed the corpus
//! with [`build_pristine_image`](pglet_testing::durability::build_pristine_image)
//! output (`deno task fuzz` / `fuzz:seed` does this) so it starts from real on-disk
//! structure — magic, two meta slots, checksummed pages — and mutates toward new
//! decoder coverage rather than flailing at the magic check.
//!
//! 1. **As mutated.** The image must reopen to a **valid committed prefix** or fail
//!    loud: [`surviving_ids`] opens and scans, and the one failure condition is a
//!    `Some` result that is not a prefix of `1..=PRISTINE_ROWS` — a torn read, the
//!    one thing a durable store must never do. This is the coverage-guided,
//!    persistent-corpus lift of `pglet/tests/crash.rs`'s seeded random-corruption
//!    sweep, sharing its exact invariant.
//! 2. **Behind valid checksums.** The same image with every leaf and overflow page's
//!    checksum re-stamped ([`restamp_leaf_and_overflow_pages`]), so a mutation there
//!    reaches the leaf codec and the overflow-chain walk instead of failing a
//!    checksum. A changed value can now read back as a different one, so no prefix is
//!    asked — only that the open and the scan return rather than panic, abort, or
//!    run without end.
//!
//! Neither leg catches a panic: a panic on an untrusted image surfaces as a libFuzzer
//! crash, a robustness finding (the engine should fail loud with an error).

use libfuzzer_sys::fuzz_target;
use pglet_testing::durability::{PRISTINE_ROWS, is_valid_prefix, surviving_ids};
use pglet_testing::store_image::restamp_leaf_and_overflow_pages;

fuzz_target!(|data: &[u8]| {
    if let Some(ids) = surviving_ids(data.to_vec()) {
        assert!(
            is_valid_prefix(&ids, PRISTINE_ROWS),
            "torn read: reopen returned a non-prefix set {ids:?} \
             (expected a prefix of 1..={PRISTINE_ROWS} or a loud failure)"
        );
    }
    let mut restamped = data.to_vec();
    restamp_leaf_and_overflow_pages(&mut restamped);
    let _ = surviving_ids(restamped);
});
