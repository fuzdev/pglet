//! Test-only helpers for pglet.
//!
//! **Dev-dependency only — NEVER link this crate from a production binary.** The
//! other crates depend on it through `dev-dependencies` (and `pglet_tests`, whose
//! own binaries are test harnesses), so it never enters a production build graph.
//! `cargo xtask check-release` turns that into an enforced invariant: this crate
//! is on its forbidden list, and every production binary's runtime closure is
//! walked for it.
//!
//! Modules are feature-gated so a leaf consumer pulls only what it needs:
//! - `block-io` — in-memory + fault-injecting `BlockIo` doubles
//!   (`block_io::MemBlockIo`) and hand-editing a store image behind valid
//!   checksums (`store_image`), and power-loss images of a recorded write log
//!   (`power_loss`); needs `pglet_storage` only.
//! - `embed` — engine assembly (`db::open_database`) + `db::ScratchDir`, and the
//!   recover-or-loud invariant the crash tests and the fuzz target share
//!   (`durability`); implies `block-io` (needs `pglet` and `pglet_sql`).
//! - `oracle` — the `oracle::MemStorage` scan-and-filter `Storage` oracle the
//!   real B+tree backend is differential-tested against (needs `pglet_sql`).
//! - `server` — spawn / wait / connect / hard-kill harness for the native
//!   `pglet_server` (needs `tokio` + `tokio-postgres`).
//!
//! Always available, with no feature: `rng` (a seedable PRNG) and `proc` (the
//! `/proc` resident-set reader).

// Test-support code: `unwrap`/`expect`/`panic` are the correct way to surface a
// broken fixture or harness loudly. The workspace restriction lints that forbid
// them in production code are relaxed here — this crate never ships in one.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

/// Seedable PRNG for reproducible fuzz/soak loops (pure std, always available).
pub mod rng;

/// `/proc/<pid>/status` resident-set reader for the soak/memory harnesses (pure
/// std + Linux, always available; degrades to `None` off Linux).
pub mod proc;

#[cfg(feature = "block-io")]
pub mod block_io;
#[cfg(feature = "embed")]
pub mod db;
#[cfg(feature = "embed")]
pub mod durability;
#[cfg(feature = "oracle")]
pub mod oracle;
#[cfg(feature = "block-io")]
pub mod power_loss;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "block-io")]
pub mod store_image;
