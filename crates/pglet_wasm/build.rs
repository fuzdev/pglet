//! Links the wasm module with a 4 MiB shadow stack in place of rustc's 1 MiB default.
//!
//! Every statement's walkers recurse once per tree level, and on this target each frame
//! takes linear-memory stack as well as the host engine's own. `pglet_sql::depth`
//! bounds the tree at `DEFAULT_MAX_DEPTH` levels, sized by the **host** stack (which a
//! module cannot raise); this makes the shadow stack the one that is *not* binding at
//! that limit — about 0.85 KiB a level on the dearest shape, so 1.7 MiB of the 4 — so
//! running out of it can no longer trap and poison the instance. The stack still sits
//! at the bottom of linear memory (`--stack-first`), which is what `--low-memory-unused`
//! in `Cargo.toml` relies on.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo:rustc-link-arg-cdylib=-zstack-size=4194304");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
