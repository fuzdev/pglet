//! Write the pristine double-meta image to a file, to seed the `double_meta_reopen`
//! fuzz corpus (`deno task fuzz:seed`). The fuzz target mutates this *valid* image,
//! so libFuzzer starts from real on-disk structure (magic, two meta slots, CRC'd
//! pages) and explores decoder branches instead of flailing at the magic check.
//!
//!   cargo run -p pglet --example gen_fuzz_seed -- <output-path>

use pglet_testing::durability::{PRISTINE_ROWS, build_pristine_image};
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: gen_fuzz_seed <output-path>");
        return ExitCode::FAILURE;
    };
    if let Some(parent) = std::path::Path::new(&path).parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("create {}: {e}", parent.display());
        return ExitCode::FAILURE;
    }
    let image = build_pristine_image(PRISTINE_ROWS);
    if let Err(e) = std::fs::write(&path, &image) {
        eprintln!("write {path}: {e}");
        return ExitCode::FAILURE;
    }
    eprintln!(
        "wrote {} bytes ({PRISTINE_ROWS} rows) to {path}",
        image.len()
    );
    ExitCode::SUCCESS
}
