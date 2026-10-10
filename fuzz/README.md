# pglet durability fuzzing

Coverage-guided (`cargo-fuzz` / libFuzzer) durability fuzzing of the on-disk
recovery path. This is the persistent-corpus, coverage-guided counterpart to the
deterministic, seeded random-corruption sweep in
`crates/pglet/tests/crash.rs` — both assert the **same** recover-or-loud invariant,
whose helpers live once in `pglet_testing::durability`.

This crate is a **standalone workspace** (the parent workspace is `crates/*`), so it
never enters the production build graph, `cargo build/test --workspace`, or the
`check:release` audit. It builds only under `cargo fuzz`.

## Requirements

- A nightly toolchain (libFuzzer needs `-Zsanitizer`): `rustup toolchain install nightly`.
- `cargo-fuzz`: `cargo install cargo-fuzz`.
- A C compiler (libfuzzer-sys builds libFuzzer via `cc`).

## Run

```bash
deno task fuzz            # seed the corpus, then fuzz for 60s
deno task fuzz:seed       # (re)write the valid seed image into the corpus

# or directly, for a longer / custom run:
cargo run -p pglet --example gen_fuzz_seed -- fuzz/corpus/double_meta_reopen/pristine
cargo +nightly fuzz run double_meta_reopen -- -max_total_time=600 -rss_limit_mb=4096
```

The corpus (`fuzz/corpus/`) and any crash artifacts (`fuzz/artifacts/`) are
git-ignored; the corpus accumulates across runs. Seed with a *valid* image
(`gen_fuzz_seed`) so libFuzzer mutates from real on-disk structure — magic, two meta
slots, CRC'd pages — instead of flailing at the magic check.

## Targets

### `double_meta_reopen`

A (mutated) `DoubleMetaStore` file image must reopen to a **valid committed prefix**
or **fail loud** — never a torn read. `surviving_ids` opens via the *checked* path
(`open_database_checked`), so a detected-corrupt image (torn/CRC-bad meta, bad magic,
undecodable schema) or a query error is a clean `None`. The single assert failure is
a `Some` result that is not a prefix of `1..=PRISTINE_ROWS` — a torn read.

A raw mutation of a data page fails that page's checksum before the page codec or
the tree reads it, so this check alone reaches little past the checksum. Each input
therefore runs a second leg: the same image with the checksum of every page whose
header names it a leaf or an overflow page re-stamped (`pglet_testing::store_image::restamp_leaf_and_overflow_pages`),
so a mutation there reaches the leaf codec, the leaf entries' chain references and
the overflow-chain walk. A changed value can then read back as a different one, so
that leg asks no prefix — only that the open and the scan return. Internal pages
keep their checksums in both legs: this leg is aimed at the leaf and chain structure.
A tree walk refuses a page it reaches twice — an internal page naming an ancestor
included — as a corrupt tree (`EDGES.md` row 205), so re-stamping internal pages
too would reach that check rather than hang; no leg does yet.

This target is deliberately **stricter** than the deterministic sweep: it does **not**
catch panics, so a panic on an untrusted image surfaces as a libFuzzer crash — a
robustness finding, since the engine should fail loud with an *error*, not panic.
