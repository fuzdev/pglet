//! pglet WASM bindings
//!
//! This crate provides WebAssembly bindings for pglet, allowing it to run in the browser.
//!
//! # Usage
//!
//! ## In-Memory Database (Main Thread)
//!
//! ```javascript
//! import init, { PgletBtree } from './pglet_wasm.js';
//!
//! await init();
//! const db = new PgletBtree();
//! db.exec("CREATE TABLE users (id INT, name TEXT)");
//! db.exec("INSERT INTO users VALUES (1, 'Alice')");
//! const result = db.query("SELECT * FROM users");
//! console.log(result.rows);
//! ```
//!
//! ## Persistent Database (Web Worker with B+tree, `opfs` feature)
//!
//! The OPFS-backed `PgletBtreePersistent` is an opt-in layer: only a build with the
//! crate's `opfs` feature on exports it (`deno task wasm:build:opfs`, a `wasm-pack build`
//! with `--features opfs`). The default build — the one the npm package ships — carries
//! neither it nor the OPFS shim.
//!
//! ```javascript
//! // worker.js
//! import init, { PgletBtreePersistent } from './pglet_wasm.js';
//!
//! await init();
//! const db = await PgletBtreePersistent.open();
//! db.exec("CREATE TABLE users (id INT, name TEXT)");
//! db.exec("INSERT INTO users VALUES (1, 'Alice')");
//! // Each autocommit write is durable immediately — it survives a reload.
//! // To batch many writes into one durable commit, wrap them in BEGIN/COMMIT.
//! ```

use wasm_bindgen::prelude::*;

mod call;
mod database;
mod error;
#[cfg(feature = "opfs")]
mod opfs_block_io;
mod param;
#[cfg(feature = "opfs")]
mod persistent;
mod result;
mod utils;

// Re-export the main types
pub use database::PgletBtree;
#[cfg(feature = "opfs")]
pub use persistent::PgletBtreePersistent;
pub use result::QueryResult;

// The wasm build runs the B+tree only, and leaves `pglet`'s `memory_storage` feature off
// so `StorageBackend` carries no `Memory` arm for it to link. This match is exhaustive
// only while that holds: a dependency edge that turns the feature back on for the wasm
// build — feature unification needs just one — fails the build here (`E0004`) rather
// than silently growing the artifact. A native build's feature set depends on what else
// the invocation selects — `cargo test -p pglet_wasm` alone leaves the feature off, a
// `--workspace` build unifies it on through the native consumers — so the check is
// gated to the wasm target, the build it is about. `deno task check:wasm-features` lints
// `pglet` and this crate for the wasm target under the wasm build's feature sets, the
// default, the stripped `--no-default-features` one and the `opfs` one.
#[cfg(target_arch = "wasm32")]
const _: fn(&pglet::StorageBackend) = |backend| match backend {
    pglet::StorageBackend::Btree(_) => {}
};

/// Initialization function called when the WASM module loads.
/// Sets up panic hook for better error messages in the browser console.
#[wasm_bindgen(start)]
pub fn main() {
    utils::set_panic_hook();
}
