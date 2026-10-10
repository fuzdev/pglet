//! The durability recovery invariant, in one place.
//!
//! "After any corruption of a committed image, a reopen recovers to a **valid
//! committed prefix** or **fails loud** — never a torn read." Both the
//! deterministic crash tests (`pglet/tests/crash.rs`, which corrupt known fault
//! windows + a seeded random sweep) and the coverage-guided fuzz target
//! (`fuzz/fuzz_targets/double_meta_reopen.rs`, which lets libFuzzer mutate the
//! image) assert this single contract through these helpers, so the two can't
//! drift on what "recover-or-loud" means.

use pglet_sql::types::Value;

use crate::block_io::MemBlockIo;
use crate::db::{open_database, open_database_checked};

/// Row count of the standard pristine image.
///
/// The deterministic crash sweep, the fuzz seed generator, and the fuzz target's
/// prefix bound all key off this one constant, so the seed and the `is_valid_prefix`
/// check can't disagree.
pub const PRISTINE_ROWS: i32 = 24;

/// Build a clean store image holding ids `1..=n` (each its own auto-commit) and
/// return its raw bytes.
///
/// A valid seed image to clone-and-corrupt (the deterministic test) or to seed the
/// fuzz corpus with (so libFuzzer mutates from real on-disk structure rather than
/// random noise). Every fourth row carries a `pad` long enough to be stored out of
/// line, so the image holds overflow chains for the corruption to land in: a scan
/// of `t` reads every chain, the `id` column's alone included.
#[must_use]
pub fn build_pristine_image(n: i32) -> Vec<u8> {
    let io = MemBlockIo::new();
    {
        let mut db = open_database(Box::new(io.clone()));
        db.execute("CREATE TABLE t (id INT, pad TEXT)")
            .expect("create table");
        for i in 1..=n {
            let pad = if i % 4 == 0 {
                char::from(b'a' + (i % 26) as u8)
                    .to_string()
                    .repeat(6_000 + i as usize)
            } else {
                String::new()
            };
            db.execute(&format!("INSERT INTO t VALUES ({i}, '{pad}')"))
                .expect("insert row");
        }
    }
    io.snapshot_bytes()
}

/// Recovered ids if `bytes` opens and scans cleanly, else `None`.
///
/// A *clean loud* outcome: a detected-corrupt open ([`open_database_checked`] →
/// `None`) or a query error (e.g. an empty image opens as a fresh store with no
/// table `t`). A non-`Int4` value decodes to the sentinel `i32::MIN`, so a torn
/// value fails [`is_valid_prefix`] rather than masquerading as a valid id.
///
/// Deliberately does **not** catch panics: the panic policy lives at the call site,
/// because the two consumers want opposite things. The deterministic crash sweep
/// catches (a decode panic = acceptable "loud"); the `double_meta_reopen` fuzz
/// target lets it surface (a panic on an untrusted image is a robustness finding —
/// the engine should fail loud with an error, not panic).
#[must_use]
pub fn surviving_ids(bytes: Vec<u8>) -> Option<Vec<i32>> {
    let mut db = open_database_checked(Box::new(MemBlockIo::from_bytes(bytes)))?;
    let res = db.execute("SELECT id FROM t ORDER BY id").ok()?;
    Some(
        res.first()?
            .rows
            .iter()
            .map(|row| match row.first() {
                Some(Value::Int4(v)) => *v,
                _ => i32::MIN,
            })
            .collect(),
    )
}

/// A valid recovered state is exactly the contiguous prefix `[1, 2, …, j]` for
/// some `0 ≤ j ≤ n` — no gaps, duplicates, reorderings, out-of-range, or torn
/// (`i32::MIN`) values.
#[must_use]
pub fn is_valid_prefix(ids: &[i32], n: i32) -> bool {
    if ids.len() as i32 > n {
        return false;
    }
    ids.iter().enumerate().all(|(i, &id)| id == i as i32 + 1)
}
