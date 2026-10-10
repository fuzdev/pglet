/// Sets up better panic messages in the browser console.
/// This should be called once when the WASM module is initialized.
///
/// **The `allow` answers a lint only one build profile raises, and the other profile
/// is why it cannot be obeyed.** With `console_error_panic_hook` off — the shape
/// `cargo clippy --workspace --no-default-features` takes — the body is empty and
/// `missing_const_for_fn` fires; with the feature on, `set_once` is not a `const fn`,
/// so making this one `const` is an `E0015` compile error. Measured both ways. The
/// *shipped* stripped wasm build re-adds the feature
/// (`--no-default-features --features console_error_panic_hook`), so it takes the
/// non-const arm too — the lint is reachable only from the clippy invocation.
#[allow(clippy::missing_const_for_fn)]
pub fn set_panic_hook() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}
