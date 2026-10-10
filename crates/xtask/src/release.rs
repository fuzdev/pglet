//! `cargo xtask check-release` — the release firewall.
//!
//! One question: **does a shipped artifact reach a dev-only crate over the
//! runtime dependency graph?** `pglet_testing` is a test toolbox — `BlockIo`
//! fault injection, the `MemStorage` oracle, a server spawn/kill harness — and
//! it is a dev-dependency of the crates that use it, so it is structurally
//! outside every shipped artifact's runtime closure. This turns that structure
//! into an enforced invariant: a `[dependencies]` edge to it (a mis-typed
//! manifest line, a "just for one test" shortcut) fails the check instead of
//! silently linking a testing implementation into something that ships.
//!
//! "Artifact" is wider than "binary": `pglet_wasm` is a `cdylib` and is the npm
//! package, so it is scanned beside the two binaries.
//!
//! ## How the graph is read
//!
//! One `cargo tree` run over the whole workspace:
//!
//! ```text
//! cargo tree --workspace -e normal --all-features --no-dedupe --target all \
//!            --prefix depth --format '|{p}'
//! ```
//!
//! (the quotes are the shell's — [`workspace_runtime_trees`] passes `|{p}` as one
//! argv element, and an unquoted `|` on a shell is a pipe)
//!
//! and each production package's own subtree is taken out of it (the output is
//! one tree per member, roots at depth 0). Every flag is load-bearing — the two
//! formatting ones included, and what each buys is stated below rather than
//! asserted:
//!
//! - `-e normal` is exactly the edge filter the question needs: dev-dependencies
//!   and build-dependencies are excluded, so only what a `cargo build` of that
//!   artifact actually links is walked.
//! - `--workspace --all-features` together activate **every feature of every
//!   workspace member**, and that is what makes the walk see an *optional*
//!   `[dependencies]` edge. `-p <pkg> --all-features` activates the selected
//!   package's own features only, so an optional edge declared by some other
//!   member is invisible to it. Measured, on a clone mutated in two steps: give
//!   `pglet_wire` an `optional = true` dependency on `pglet_testing` behind a
//!   new `pglet_wire` feature, then give `pglet_tests` a `pglet_wire` dependency
//!   requesting that feature (`pglet_tests` has no `pglet_wire` edge on the real
//!   tree — zero rows in its closure — so the requester has to be added, and
//!   `pglet_wire` is the crate the mutation can use because `pglet_sql` would
//!   close a cycle). `cargo tree -p pglet_server -e normal --all-features`
//!   printed `pglet_testing` zero times and the per-package walk reported `OK`,
//!   while this walk fails naming
//!   `pglet_server -> pglet_wire -> pglet_testing`. Nothing hypothetical stands
//!   between that and a shipped binary, and the stronger measurement is the
//!   reason: with the requester present the **default-feature** graph — `cargo
//!   tree --workspace -e normal --no-dedupe`, no `--all-features`, which is what
//!   `cargo build --release` at the root links — carries `pglet_testing` under
//!   `pglet_server` at depth 2 too. (`deno task build` is that build, and the
//!   root manifest sets `resolver = "2"` explicitly, so a workspace-wide build
//!   unifies features across members for one target.) This is also what bounds
//!   the feature axis — on **three** premises, of which the third is a fact
//!   about this tree rather than a structural one. A forbidden crate is by
//!   construction a workspace package (an entry naming none fails below); a
//!   registry package cannot carry a `path` or `git` dependency; and every crate
//!   here is `publish = false`, so no registry package exists that could name one
//!   by version either. Those three give "an edge into a forbidden crate is
//!   declared by a workspace member or it is not declared" only while every
//!   non-member node in the walked graph *is* a registry package — measured true
//!   today (`Cargo.lock`'s only source-less entries are the workspace members;
//!   every other entry carries a `registry+…` source, and no manifest here
//!   declares a `git =` dependency), and it is that premise a future git
//!   dependency would take away: a git node is a non-member whose own optional
//!   edges `--workspace --all-features` does not activate. The *guard* would
//!   still name a reached `pglet_testing` — it is this bound on the feature axis
//!   that would lapse, and re-establishing it would mean re-reading the lockfile.
//! - `--no-dedupe` because with several roots `cargo tree` prints a node it has
//!   already expanded under an earlier root as `(*)` with its subtree elided —
//!   measured on this tree under otherwise identical flags, `pglet_server`'s
//!   subtree is 12 node rows deduped against 2112 expanded (root included, blank
//!   separators excluded) — so a forbidden crate under an elided subtree would
//!   not be in the output at all.
//! - `--target all` so an edge behind a `[target.'cfg(…)'.dependencies]` table
//!   is walked whatever platform the check runs on. `pglet_wasm` ships built for
//!   `wasm32-unknown-unknown`, and `pglet_sql` has a
//!   `cfg(target_arch = "wasm32")` table: measured, `js-sys` appears under it
//!   with `--target all` and not without.
//! - `--prefix depth` gives the depth of each node, and two things read it:
//!   [`violations`] turns it into the chain that reaches a forbidden crate, and
//!   [`split_trees`] uses a depth-0 row as the only thing separating one member's
//!   closure from the next. The second is the load-bearing one — a walk whose
//!   roots could not be told apart would report every violation against one
//!   member, naming the wrong artifact — and it is not left to trust: dropping
//!   the flag does not silently merge the trees, because [`parse_nodes`] keeps
//!   only a `<digits>|…` row. Measured, on the otherwise identical run: of its
//!   15 705 lines, **0** match that shape (the roots lose the depth entirely,
//!   the rest carry `├── ` where it was), so `trees` comes back empty and
//!   `workspace_runtime_trees` fails with "produced no dependency trees" —
//!   `EX_UNAVAILABLE`, not a wrong answer.
//! - `--format |{p}` is `{p}` — cargo's default, so it changes no content — with
//!   a separator in front of it. `--prefix depth` abuts the depth to the package
//!   with nothing between them, so a depth-1 row for a package named `3des`
//!   reads as depth 13 of a package `des`: a wrong ancestor chain, and a missed
//!   violation if such a package were the forbidden crate's parent. The `|` is
//!   what [`parse_nodes`] splits on, and no cargo package name may contain one.
//!   Measured: the run with the separator is the same 15 705 lines and the same
//!   11 roots as the run without it.
//!
//! Over-approximation is the deliberate posture on both the feature and the
//! target axis: a missed edge is a silently linked test crate, while a spurious
//! one is a loud failure a human reads. That posture is live on this tree, not
//! theoretical — measured with `--format '{p} [{f}]'`, `pglet_wasm`'s scanned
//! closure carries `pglet [default,file_store,index_acceleration]` and
//! `pglet_storage [default,file_store]`, the native `std::fs` configuration that
//! the `default-features = false` pin on the wasm edge exists to keep out. No
//! production closure reaches `pglet_testing` today (zero rows under each of the
//! three roots), so the shape to expect one day is a forbidden crate reached
//! *only* through `pglet_storage/file_store` and reported against `pglet_wasm`,
//! whose real build cannot link it. Read such a failure as "some configuration
//! of this member reaches it", never as "the shipped wasm links it".
//!
//! ## Classification is forced, not opted out of
//!
//! Every workspace package that produces an artifact — a binary target or a
//! linkable library (`cdylib`/`staticlib`) — must appear in exactly one of
//! [`PRODUCTION_ARTIFACT_PACKAGES`] or [`DEV_ONLY_ARTIFACT_PACKAGES`]. A new one
//! in neither fails the check by name. This is the inverse of an opt-out marker
//! in each crate's own manifest: an unclassified new crate there is silently
//! *scanned or not* depending on which default it lands in, and the failure mode
//! of a marker is that whoever adds the crate writes it without deciding. The
//! lists also fail when an entry names no package, so a rename cannot quietly
//! empty the scan.
//!
//! *Which* packages that check ranges over is the workspace's member set, and
//! that is enforced rather than assumed: [`member_set_gaps`] compares the walk's
//! depth-0 roots against the `crates/` scan in both directions. A member declared
//! outside `crates/` is a root nobody classifies; a manifest under `crates/` that
//! is not a member is classified but never walked. Each fails by name.
//!
//! ### What counts as producing an artifact
//!
//! [`produces_artifact`] tracks cargo's binary discovery — a declared `bin` table
//! and the three auto-discovered layouts (`src/main.rs`, `src/bin/<name>.rs`,
//! `src/bin/<name>/main.rs`) — plus a `[lib] crate-type` naming `cdylib` or
//! `staticlib`. Three deliberate readings:
//!
//! - The manifest is read as lines rather than parsed as TOML, so the `bin`
//!   table is recognized by **spelling**. Two *structures* declare it — the
//!   `[[bin]]` array-of-tables header, and the top-level inline array
//!   `bin = [{ … }]`, which is the same structure with no header — and TOML lets
//!   the key `bin` in either of them be written bare, `"bin"`, or `'bin'`
//!   interchangeably. [`declares_bin_table`] therefore squeezes whitespace *and*
//!   those key quotes out of the line before comparing, so the quoting is not
//!   part of what it recognizes; each of the six accepting spellings that
//!   collapses onto the two forms was measured building a real binary, and each
//!   is pinned by a unit test. Where a table's `path` points is never read, and
//!   that is exactly why the spelling carries the weight: a `path` outside `src/`
//!   leaves nothing for the three layout arms to find, so the table arm is the
//!   only one that can fire and a spelling it does not know is a crate nobody is
//!   asked to classify. What it still does not know is a key written with a TOML
//!   **string escape** — measured, cargo accepts `[["b\u0069n"]]` and builds a
//!   `bin` target from it, and no amount of squeezing sees that without decoding
//!   the escape. A spelling missing from the list is the standing risk of a
//!   line-shaped reader; rehearse a new one against a real `cargo build` before
//!   trusting the scan.
//! - `autobins = false` (which switches cargo's auto-discovery off) is **not**
//!   read. A package with that key and an auto-discoverable file is still
//!   required to be classified, so this over-approximates in the direction of
//!   demanding a human decision rather than skipping a scan.
//! - A filesystem scan and not `cargo metadata` keeps the firewall
//!   dependency-free — no JSON parser in the tree of the thing that polices the
//!   tree — at the cost of tracking cargo's discovery rules by hand. The `tests`
//!   module pins each shape over a synthetic directory layout, so a shape dropped
//!   in a refactor reddens.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::runner::repo_root;

/// The packages this workspace ships: the server, the wasm `cdylib` that becomes
/// the npm package, and the bench harness (which must measure the shipped engine
/// rather than a test-instrumented one). Their runtime closures are what gets
/// scanned.
const PRODUCTION_ARTIFACT_PACKAGES: &[&str] = &["pglet_server", "pglet_wasm", "pglet_bench"];

/// Packages whose artifacts are development tooling and are never shipped —
/// `xtask` is this program, `pglet_tests` is the test-binary crate. Listed so
/// the completeness check below can tell "deliberately unscanned" from "nobody
/// classified it".
const DEV_ONLY_ARTIFACT_PACKAGES: &[&str] = &["xtask", "pglet_tests"];

/// Crates a shipped artifact must not reach over the runtime graph.
///
/// `pglet_testing` is the dev-only test toolbox. Keeping this a list rather than
/// a single name is not speculation: the check below refuses a name that
/// resolves to no workspace package, so an entry added for a crate that later
/// goes away fails loudly instead of becoming a no-op forbid.
const FORBIDDEN_CRATES: &[&str] = &["pglet_testing"];

/// Clean.
const EXIT_OK: u8 = 0;
/// A shipped artifact links a forbidden crate, a package is unclassified, or a
/// manifest is not in a shape [`package_name`] can read: everything the check
/// needed to run was there, and the input violates what the check requires.
/// `EX_DATAERR`.
const EXIT_POLICY_VIOLATION: u8 = 65;
/// `cargo tree` could not be run or its output could not be read, or `crates/`,
/// or a manifest under it, could not be read off the filesystem.
/// `EX_UNAVAILABLE`.
const EXIT_TOOLING_FAILURE: u8 = 69;

/// The `cargo xtask check-release` entry point: [`audit`] over [`repo_root`],
/// with the code it returns turned into an `ExitCode`.
pub fn run() -> ExitCode {
    ExitCode::from(audit(&repo_root()))
}

/// Run the audit and report. Prints its own diagnosis; the caller only forwards
/// the code.
///
/// The root is a parameter and the code is a `u8` so that a test can drive the
/// whole path over a synthetic `crates/`: the scan, the failure it produces,
/// and the code that failure carries. That is what makes the exit-code split a
/// guard rather than two assertions about the enum — measured, with this
/// forwarding replaced by a fixed `EXIT_TOOLING_FAILURE`, the enum-only test
/// stayed green.
fn audit(root: &Path) -> u8 {
    let packages = match workspace_packages(root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("check-release: {e}");
            return e.exit_code();
        }
    };

    let trees = match workspace_runtime_trees(root) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("check-release: {e}");
            return EXIT_TOOLING_FAILURE;
        }
    };

    let mut policy_failures = Vec::new();
    policy_failures.extend(classification_gaps(&packages));
    policy_failures.extend(member_set_gaps(&packages, &trees));

    for forbidden in FORBIDDEN_CRATES {
        if !packages.iter().any(|p| p.name == *forbidden) {
            policy_failures.push(format!(
                "forbidden crate `{forbidden}` resolves to no package in this workspace, so the \
                 forbid guards nothing — remove the entry or fix the name"
            ));
        }
    }

    for package in PRODUCTION_ARTIFACT_PACKAGES {
        match trees.iter().find(|(name, _)| name == package) {
            Some((_, nodes)) => policy_failures.extend(violations(package, nodes)),
            None => policy_failures.push(format!(
                "`{package}` is scanned as a production artifact but is not a root of the \
                 workspace dependency tree — either a rename left the scan pointing at nothing, \
                 or the package is no longer a workspace member (`exclude`d in the root \
                 manifest, or outside its `members` globs)"
            )),
        }
    }

    if policy_failures.is_empty() {
        println!(
            "OK: no shipped artifact reaches a forbidden crate over the runtime graph\n  \
             scanned: {}\n  forbidden: {}",
            PRODUCTION_ARTIFACT_PACKAGES.join(", "),
            FORBIDDEN_CRATES.join(", "),
        );
        return EXIT_OK;
    }

    eprintln!("check-release FAILED");
    for failure in &policy_failures {
        eprintln!("  {failure}");
    }
    EXIT_POLICY_VIOLATION
}

/// A workspace member, as read off the filesystem.
struct WorkspacePackage {
    name: String,
    /// Whether the package produces at least one shipped-shape target — a binary
    /// or a linkable library. See [`produces_artifact`].
    produces_artifact: bool,
}

/// Why the `crates/` scan could not produce the workspace's package list.
///
/// The two arms exit differently, and the split is the whole reason this is not
/// a bare `String`: the filesystem refusing to be read is a tooling failure
/// (`EX_UNAVAILABLE`), while a manifest [`package_name`] cannot read is
/// policy-shaped input — the tool read the file fine and the file is not in a
/// shape this check can work with, which is what `EX_DATAERR` already means.
#[derive(Debug)]
enum ScanFailure {
    /// `crates/`, or a manifest under it, could not be read off the filesystem.
    Unreadable(String),
    /// A manifest was read and no `[package]` `name` could be found in it.
    NoPackageName(String),
}

impl ScanFailure {
    /// The process exit code this failure reports.
    const fn exit_code(&self) -> u8 {
        match self {
            Self::Unreadable(_) => EXIT_TOOLING_FAILURE,
            Self::NoPackageName(_) => EXIT_POLICY_VIOLATION,
        }
    }
}

impl std::fmt::Display for ScanFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(message) | Self::NoPackageName(message) => f.write_str(message),
        }
    }
}

/// Read the workspace members out of `crates/`.
///
/// Deliberately a filesystem scan and not `cargo metadata`: the manifest's
/// `members = ["crates/*"]` is the definition, this reads the same thing, and it
/// keeps the audit dependency-free — a release firewall that needs a JSON parser
/// in its own dependency tree is an odd thing to trust. That the two agree is
/// not left to the glob: [`member_set_gaps`] compares this scan against the
/// walk's own root list.
fn workspace_packages(root: &Path) -> Result<Vec<WorkspacePackage>, ScanFailure> {
    let crates_dir = root.join("crates");
    let entries = std::fs::read_dir(&crates_dir).map_err(|e| {
        ScanFailure::Unreadable(format!("cannot read {}: {e}", crates_dir.display()))
    })?;

    let mut packages = Vec::new();
    for entry in entries {
        let dir = entry
            .map_err(|e| {
                ScanFailure::Unreadable(format!("cannot read {}: {e}", crates_dir.display()))
            })?
            .path();
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest).map_err(|e| {
            ScanFailure::Unreadable(format!("cannot read {}: {e}", manifest.display()))
        })?;
        let name = package_name(&text).ok_or_else(|| {
            ScanFailure::NoPackageName(format!(
                "could not read a `[package]` `name` out of {} — this reader is line-shaped, \
                 not a TOML parser, and the file may well hold one it cannot see. The shapes \
                 measured to defeat it: a `[package]` table declared by dotted keys \
                 (`package.name = \"…\"`), a name written as a multi-line string \
                 (`\"\"\"…\"\"\"` or `'''…'''`), and a `[`…`]` continuation line inside \
                 `[package]` above `name` (a quoted array element such as `\"[db]\"` alone \
                 on a line that does not end in a comma, which squeezes to a table header's \
                 shape — the same element followed by a comma does not)",
                manifest.display()
            ))
        })?;
        packages.push(WorkspacePackage {
            name,
            produces_artifact: produces_artifact(&text, &dir),
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(packages)
}

/// Whether a package produces something this workspace could ship.
///
/// The binary half tracks cargo's four shapes: a declared `bin` table, and the
/// three auto-discovered layouts `src/main.rs`, `src/bin/<name>.rs`, and
/// `src/bin/<name>/main.rs`. The library half is a `[lib] crate-type` naming
/// `cdylib` or `staticlib` — an `rlib`/`lib` is only ever linked into another
/// workspace target and is scanned there.
///
/// A *shape* is not a *spelling*: the `bin`-table shape has several accepted
/// spellings and [`declares_bin_table`] is where they are enumerated. See the
/// module docs for which ones it knows, which measured one it does not, why that
/// list is the load-bearing part, and what this does with `autobins = false`
/// (not read — it over-approximates).
fn produces_artifact(manifest: &str, dir: &Path) -> bool {
    declares_bin_table(manifest)
        || declares_linkable_lib(manifest)
        || dir.join("src/main.rs").is_file()
        || bin_dir_has_target(&dir.join("src/bin"))
}

/// Whether the manifest declares a `bin` array of tables.
///
/// A line goes through [`manifest_line_squeezed`] — cut at its first `#`, then
/// emptied of whitespace and of TOML's optional key quotes — leaving two
/// accepted forms:
///
/// - `[[bin]]` — what every spelling of the array-of-tables header squeezes to;
/// - anything starting `bin=[` — what every spelling of the inline array
///   `bin = [{ name = "…", path = "…" }]` squeezes to, which builds the same
///   target and never produces a header at all. A multi-line inline array
///   matches on its first line.
///
/// Squeezing the quotes is what keeps the *quoting* of the key out of the
/// recognition. TOML writes a bare key and a quoted key interchangeably, and
/// cargo builds a real binary from every one of `[[bin]]`, `[["bin"]]`,
/// `[['bin']]`, top-level `bin = [{ … }]`, `"bin" = [{ … }]` and
/// `'bin' = [{ … }]` — measured, `cargo metadata` and `cargo build` both, on
/// probe crates whose `path` points outside `src/` so no layout arm can fire.
/// All six collapse onto the two forms above. What squeezing cannot reach is a
/// key spelled with a TOML string **escape**: cargo builds a binary from
/// `[["b\u0069n"]]` too (measured the same way), and this reader answers `false`
/// for it. That residual is the shape of a line-shaped reader, and it is why the
/// enumeration is kept where the code is.
///
/// The inline form builds a binary only at the manifest's **top level**; the same
/// key written inside `[package]` is an ignored `package.bin` and cargo builds
/// nothing from it (measured, both bare and quoted, against a real
/// `cargo build`). This reader is line-shaped and matches either position, and
/// the `bin=[` prefix over-matches further by construction — a `bin` key holding
/// an array in some other table satisfies it, and since the quotes come out
/// before the prefix test, so does one holding a *string* that begins with `[`
/// (measured: `bin = "[str]"` under `[package.metadata]`, which the reader
/// answered `false` for before the quotes were squeezed). Over-approximation is the
/// safe direction here: it costs one entry in a classification list, where a miss
/// ships an unscanned artifact.
fn declares_bin_table(manifest: &str) -> bool {
    manifest_lines(manifest).any(|line| {
        let squeezed = manifest_line_squeezed(line);
        squeezed == "[[bin]]" || squeezed.starts_with("bin=[")
    })
}

/// A manifest's lines, with a leading byte-order mark taken off the first one.
///
/// The third lexical rule the two line readers owe each other, and the one that
/// only bites on line 1. Cargo accepts a single UTF-8 BOM (`EF BB BF`) at the
/// start of a manifest: measured, it reads such a file and builds it. Two
/// neighbouring shapes it refuses are what make one strip enough — measured,
/// both exit 101, a doubled mark (`key with no value, expected =` at 1:3) and a
/// mark opening a later key line (`invalid unquoted key` at 3:1). The mark sits
/// inside the first line, so a `[package]` or `[[bin]]` header written there
/// squeezes to `\u{feff}[package]` and matches nothing. Both readers were blind
/// to it, in different ways. [`package_name`] answered `None`, which
/// [`workspace_packages`] turns into a red gate on a manifest cargo builds.
/// [`declares_bin_table`] missed a `[[bin]]` header written on line 1, and that
/// one is **silent** — the name reader does not cover for it, because a header
/// on line 1 puts `[package]` further down where the name reads fine. Measured,
/// on a real member: BOM, then `[[bin]]` with a `path` outside `src/`, then
/// `[package]`. Cargo builds the binary; the pre-strip check printed `OK` and
/// exited 0 with that crate unclassified and unscanned.
fn manifest_lines(manifest: &str) -> std::str::Lines<'_> {
    manifest
        .strip_prefix('\u{feff}')
        .unwrap_or(manifest)
        .lines()
}

/// The code part of a manifest line: everything before its first `#`.
///
/// The lexical rule both line readers in this file owe each other. A trailing
/// comment is legal TOML in every position either of them looks at — after a
/// table header, and after a `name = "…"` value — and a reader that does not cut
/// it reads the comment as part of what it is looking for. Each reader has
/// shipped that bug once.
///
/// A `#` inside a quoted *value* is cut too, and neither caller can be made
/// wrong by that. [`declares_bin_table`] reads a key position, where a `#` can
/// only shorten a line that was never going to match; and a cargo package name
/// may not contain one — measured, cargo refuses `name = "ha#sh"` with
/// "invalid character `#` in package name".
fn manifest_line_code(line: &str) -> &str {
    line.split_once('#').map_or(line, |(code, _)| code)
}

/// A manifest line's code part with whitespace and TOML's optional key quotes
/// removed, so neither decides what a reader recognizes.
fn manifest_line_squeezed(line: &str) -> String {
    squeezed_key(manifest_line_code(line))
}

/// `text` with whitespace and TOML's optional key quotes (`"` and `'`) removed —
/// the normal form both readers compare a *key* in.
fn squeezed_key(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect()
}

/// The contents of the single-line TOML string `value` starts with, or `None`
/// if it does not start with one this reader reads.
///
/// Both quote styles: a basic `"…"` and a literal `'…'` are the same value to
/// cargo (measured — it accepts `name = 'probe'`). Escapes inside a basic string
/// are not decoded, and no cargo package name needs one.
///
/// A `"""` or `'''` opener is a multi-line string and is **declined** rather
/// than read: taking the text between the first two quotes would answer
/// `Some("")` for `name = """probe"""`, a name no package has, and cargo accepts
/// that manifest (measured). Declining sends it to the one caller's `None` path,
/// where the failure names the shape.
fn toml_string_value(value: &str) -> Option<String> {
    let quote = value.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &value[quote.len_utf8()..];
    if rest.starts_with(quote) && rest[quote.len_utf8()..].starts_with(quote) {
        return None;
    }
    let end = rest.find(quote)?;
    Some(rest[..end].to_owned())
}

/// Whether a `crate-type` value names a linkable library — `cdylib` or
/// `staticlib`.
///
/// Read from the key to the closing `]` so a value spread over several lines is
/// seen; every `crate-type` occurrence is checked, so a manifest that grows a
/// second one does not hide behind the first.
fn declares_linkable_lib(manifest: &str) -> bool {
    manifest.match_indices("crate-type").any(|(at, _)| {
        let rest = &manifest[at..];
        let value = rest.find(']').map_or(rest, |end| &rest[..=end]);
        value.contains("cdylib") || value.contains("staticlib")
    })
}

/// Whether `src/bin/` holds at least one auto-discovered binary: a `.rs` file
/// directly in it, or a subdirectory holding a `main.rs`. A missing directory is
/// not an error — most crates have none.
fn bin_dir_has_target(bin_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(bin_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            return path.join("main.rs").is_file();
        }
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"))
    })
}

/// Pull `name = "…"` from a manifest's `[package]` table.
///
/// Read as lines rather than parsed as TOML for the same dependency-free reason
/// as [`workspace_packages`], and through the same lexical rules as
/// [`declares_bin_table`], which is the point: the two readers read the same
/// TOML, and a rule learned by one is owed to the other.
///
/// - a leading byte-order mark is off the text before the first line is looked
///   at ([`manifest_lines`]);
/// - the line is cut at its first `#`, so a legal trailing comment
///   (`name = "pglet_wire" # the wire codec`) is not read as part of the name;
/// - the key is compared squeezed of whitespace and quotes, so `"name"` and
///   `name` are one key;
/// - the value must *begin* with a TOML string and the name is that string's
///   contents ([`toml_string_value`]), `'…'` as well as `"…"` — rather than a
///   `trim_matches('"')` that strips whatever quotes happen to sit at the ends.
///
/// Which table the line is in is tracked rather than assumed. "The first
/// `name =` in the file is the package's" is false for at least two shapes cargo
/// accepts: a `[[bin]]` table above `[package]`, and the top-level multi-line
/// `bin = [{` / `name = "…"` — both put a `name` line ahead of `[package]`, and
/// reading either yields the *binary's* name for the package. Tracking the
/// table answers those two and not a third, where the decoy sits inside
/// `[package]` itself: a `name = "…"` line within a multi-line string value
/// above the real one — `description = """` / `name = "evil"` / `"""` /
/// `name = "realname"` — is read here as `evil` where cargo reads `realname`
/// (measured, `cargo metadata`). For a workspace member a misread name is not
/// silent (the walk's roots and this scan then disagree — see
/// [`member_set_gaps`]), but it is a red gate on a manifest cargo builds,
/// pointing at a repair the manifest does not need.
///
/// A table header is recognized as a squeezed line that both opens and closes
/// with a bracket. That is a *shape*, not a proof, and it is not one only a
/// header has: a continuation line inside an array value reads as a header
/// whenever it carries the same shape, and squeezing the quotes makes such lines
/// out of ordinary strings — measured, `keywords = [` / `"[db]"` / `]` inside
/// `[package]` above `name` squeezes `"[db]"` to `[db]`, clears `in_package`,
/// and the answer is `None`. `None` is the loud direction: [`workspace_packages`]
/// fails naming the file and the shapes known to defeat this reader, rather than
/// claiming the file holds no name.
fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest_lines(manifest) {
        let code = manifest_line_code(line);
        let squeezed = manifest_line_squeezed(line);
        if squeezed.starts_with('[') && squeezed.ends_with(']') {
            in_package = squeezed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = code.split_once('=') else {
            continue;
        };
        if squeezed_key(key) != "name" {
            continue;
        }
        if let Some(name) = toml_string_value(value.trim()) {
            return Some(name);
        }
    }
    None
}

/// Which artifact-producing packages nobody classified, and which
/// classifications name a package that is not there.
fn classification_gaps(packages: &[WorkspacePackage]) -> Vec<String> {
    let mut gaps = Vec::new();

    let production: BTreeSet<&str> = PRODUCTION_ARTIFACT_PACKAGES.iter().copied().collect();
    let dev_only: BTreeSet<&str> = DEV_ONLY_ARTIFACT_PACKAGES.iter().copied().collect();

    for package in packages.iter().filter(|p| p.produces_artifact) {
        let name = package.name.as_str();
        match (production.contains(name), dev_only.contains(name)) {
            (true, false) | (false, true) => {}
            (false, false) => gaps.push(format!(
                "package `{name}` produces a shipped-shape artifact but is in neither \
                 PRODUCTION_ARTIFACT_PACKAGES nor DEV_ONLY_ARTIFACT_PACKAGES — classify it in \
                 crates/xtask/src/release.rs (an unclassified artifact is an unscanned one)"
            )),
            (true, true) => gaps.push(format!(
                "package `{name}` is listed as both production and dev-only"
            )),
        }
    }

    let present: BTreeSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    for listed in PRODUCTION_ARTIFACT_PACKAGES
        .iter()
        .chain(DEV_ONLY_ARTIFACT_PACKAGES)
    {
        if !present.contains(listed) {
            gaps.push(format!(
                "`{listed}` is classified in crates/xtask/src/release.rs but no such package \
                 exists — a rename left the scan pointing at nothing"
            ));
        }
    }
    gaps
}

/// Where the set the classification check ranges over and the set the walk
/// actually covers disagree.
///
/// [`workspace_packages`] reads `crates/`; the walk's depth-0 roots are cargo's
/// own member list. They coincide today because `members = ["crates/*"]`, and
/// nothing but this comparison holds them together: a member declared under some
/// other path is walked but never classified (so it can ship unscanned — the
/// forced-classification failure mode, reached from the other side), and a
/// manifest under `crates/` that cargo does not treat as a member is classified
/// but never walked.
///
/// What the comparison knows is exactly that the two *name sets* differ. It has
/// not looked for a manifest, read the root `members` globs, or asked cargo
/// anything, so each message names the causes it can name and asserts nothing
/// else — a name [`package_name`] misread is one of them, and puts the same
/// crate on both sides at once under two different names.
fn member_set_gaps(
    packages: &[WorkspacePackage],
    trees: &[(String, Vec<TreeNode>)],
) -> Vec<String> {
    let scanned: BTreeSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    let roots: BTreeSet<&str> = trees.iter().map(|(name, _)| name.as_str()).collect();

    let mut gaps = Vec::new();
    for root in roots.difference(&scanned) {
        gaps.push(format!(
            "workspace member `{root}` is a root of the `cargo tree --workspace` walk, and the \
             `crates/` scan produced no package of that name — so nothing asks it to be \
             classified and it could ship unscanned. The causes this check can name: its \
             manifest is not *directly* under `crates/` (the scan reads `crates/<pkg>/\
             Cargo.toml` and goes no deeper, so a member nested one level further down is \
             missed as well — move it, or widen `workspace_packages` in \
             crates/xtask/src/release.rs), or its manifest is where the scan looks and \
             `package_name` read a different name out of it"
        ));
    }
    for package in scanned.difference(&roots) {
        gaps.push(format!(
            "package `{package}` was found by the `crates/` scan but is not a root of the `cargo \
             tree --workspace` walk, so nothing walks its closure. The causes this check can \
             name: an `exclude` entry in the root manifest, a `members` glob that no longer \
             matches it, or a name `package_name` misread (both readers here are line-shaped, \
             not a TOML parser)"
        ));
    }
    gaps
}

/// One node of a `cargo tree` walk: its depth and its package name.
struct TreeNode {
    depth: usize,
    name: String,
}

/// Every workspace member's runtime closure, as `(root package, depth-tagged
/// pre-order walk)` pairs — one `cargo tree` run, split at its depth-0 rows.
///
/// See the module docs for why each flag of that run is there.
fn workspace_runtime_trees(root: &Path) -> Result<Vec<(String, Vec<TreeNode>)>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let args = [
        "tree",
        "--workspace",
        "-e",
        "normal",
        "--all-features",
        "--no-dedupe",
        "--target",
        "all",
        "--prefix",
        "depth",
        "--format",
        "|{p}",
    ];
    let output = Command::new(&cargo)
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run `{cargo} {}`: {e}", args.join(" ")))?;

    if !output.status.success() {
        return Err(format!(
            "`{cargo} {}` failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trees = split_trees(&parse_nodes(&stdout)?);
    if trees.is_empty() {
        return Err(format!(
            "`{cargo} {}` produced no dependency trees",
            args.join(" ")
        ));
    }
    Ok(trees)
}

/// Parse `--prefix depth --format |{p}` rows into depth-tagged nodes, dropping
/// anything that is not `<digits>|<package> …` (blank separators, notes).
///
/// The `|` is why the format string is spelled out: `--prefix depth` writes the
/// depth flush against the package, so without a separator a digit-initial
/// package name is read as more depth digits (see the module docs).
fn parse_nodes(stdout: &str) -> Result<Vec<TreeNode>, String> {
    let mut nodes = Vec::new();
    for line in stdout.lines() {
        let Some((depth, rest)) = line.split_once('|') else {
            continue;
        };
        if depth.is_empty() || !depth.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let depth = depth
            .parse()
            .map_err(|e| format!("unreadable `cargo tree` depth in {line:?}: {e}"))?;
        let Some(name) = rest.split_whitespace().next() else {
            continue;
        };
        nodes.push(TreeNode {
            depth,
            name: name.to_owned(),
        });
    }
    Ok(nodes)
}

/// Split a `--workspace` walk into one closure per root. A depth-0 row starts a
/// new tree; rows before the first one (there are none in practice) are dropped.
fn split_trees(nodes: &[TreeNode]) -> Vec<(String, Vec<TreeNode>)> {
    let mut trees: Vec<(String, Vec<TreeNode>)> = Vec::new();
    for node in nodes {
        if node.depth == 0 {
            trees.push((node.name.clone(), Vec::new()));
        }
        if let Some((_, current)) = trees.last_mut() {
            current.push(TreeNode {
                depth: node.depth,
                name: node.name.clone(),
            });
        }
    }
    trees
}

/// Every forbidden crate reachable in `nodes`, reported with the chain that
/// reaches it.
///
/// The chain comes from the depth column: a pre-order walk's ancestors are the
/// most recent node at each shallower depth, so tracking one name per depth
/// gives the path that arrived at the current node. Reported once per forbidden
/// crate — the first path found is a true path, and printing every path to the
/// same crate buries the diagnosis.
fn violations(package: &str, nodes: &[TreeNode]) -> Vec<String> {
    let mut ancestors: Vec<&str> = Vec::new();
    let mut reported: BTreeSet<&str> = BTreeSet::new();
    let mut out = Vec::new();

    for node in nodes {
        ancestors.truncate(node.depth);
        let Some(forbidden) = FORBIDDEN_CRATES.iter().find(|f| **f == node.name) else {
            ancestors.push(node.name.as_str());
            continue;
        };
        if reported.insert(forbidden) {
            let mut chain = String::new();
            for step in &ancestors {
                let _ = write!(chain, "{step} -> ");
            }
            chain.push_str(&node.name);
            out.push(format!(
                "shipped artifact `{package}` reaches forbidden crate `{forbidden}` over the \
                 runtime graph: {chain}"
            ));
        }
        ancestors.push(node.name.as_str());
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;

    fn nodes(rows: &[(usize, &str)]) -> Vec<TreeNode> {
        rows.iter()
            .map(|(depth, name)| TreeNode {
                depth: *depth,
                name: (*name).to_owned(),
            })
            .collect()
    }

    /// A throwaway crate layout under `target/`, so a synthetic-shape test never
    /// touches a shared temp path another process could be using.
    struct CrateLayout {
        dir: std::path::PathBuf,
    }

    impl CrateLayout {
        fn new(tag: &str) -> Self {
            let dir = repo_root()
                .join("target/xtask_release_tests")
                .join(format!("{tag}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create the synthetic crate dir");
            Self { dir }
        }

        fn file(&self, relative: &str) -> &Self {
            self.file_with(relative, "")
        }

        fn file_with(&self, relative: &str, contents: &str) -> &Self {
            let path = self.dir.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create the synthetic parent dir");
            }
            std::fs::write(&path, contents).expect("write the synthetic file");
            self
        }
    }

    impl Drop for CrateLayout {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const LIB_ONLY: &str = "[package]\nname = \"probe\"\n";

    #[test]
    fn a_clean_closure_has_no_violations() {
        let found = violations("pglet_server", &nodes(&[(0, "pglet_server"), (1, "pglet")]));
        assert!(found.is_empty(), "unexpected: {found:?}");
    }

    #[test]
    fn a_reached_forbidden_crate_is_reported_with_its_chain() {
        let found = violations(
            "pglet_server",
            &nodes(&[
                (0, "pglet_server"),
                (1, "pglet"),
                (2, "pglet_testing"),
                (1, "pglet_wire"),
            ]),
        );
        assert_eq!(found.len(), 1, "one violation: {found:?}");
        assert!(
            found[0].ends_with("pglet_server -> pglet -> pglet_testing"),
            "chain names every hop: {}",
            found[0]
        );
    }

    #[test]
    fn a_sibling_subtree_does_not_leak_into_the_chain() {
        // The depth column is the only ancestry signal, so a chain built by
        // appending every visited node would walk back out of a finished
        // subtree and print a path that does not exist.
        let found = violations(
            "pglet_server",
            &nodes(&[
                (0, "pglet_server"),
                (1, "pglet"),
                (2, "pglet_sql"),
                (3, "pglet_core"),
                (1, "pglet_testing"),
            ]),
        );
        assert_eq!(found.len(), 1, "one violation: {found:?}");
        assert!(
            found[0].ends_with("pglet_server -> pglet_testing"),
            "the finished subtree is not in the chain: {}",
            found[0]
        );
    }

    #[test]
    fn the_package_name_is_read_from_the_package_table() {
        let manifest = "[package]\nname = \"pglet_server\"\nversion.workspace = true\n\n\
                        [[bin]]\nname = \"pglet_server\"\n";
        assert_eq!(package_name(manifest).as_deref(), Some("pglet_server"));
    }

    #[test]
    fn the_package_name_survives_the_lexical_shapes_this_reader_claims() {
        // The first five are legal TOML that cargo builds, and each once yielded
        // a name no package has — which `member_set_gaps` turns into a red gate
        // whose message points at a repair the manifest does not need. The last
        // two pin the table rule that makes the fourth and fifth work.
        assert_eq!(
            package_name("[package]\nname = \"pglet_wire\" # the wire codec\n").as_deref(),
            Some("pglet_wire"),
            "a trailing comment is not part of the name"
        );
        assert_eq!(
            package_name("[package]\nname = 'pglet_wire'\n").as_deref(),
            Some("pglet_wire"),
            "a literal string is a string"
        );
        assert_eq!(
            package_name("[\"package\"]\n\"name\" = \"pglet_wire\"\n").as_deref(),
            Some("pglet_wire"),
            "quoting a key does not change it"
        );
        assert_eq!(
            package_name(
                "bin = [{\nname = \"probe_bin\",\npath = \"other/place.rs\"\n}]\n\n\
                 [package]\nname = \"probe\"\n"
            )
            .as_deref(),
            Some("probe"),
            "a `name` line above `[package]` is some other table's"
        );
        assert_eq!(
            package_name("[[bin]]\nname = \"probe_bin\"\n\n[package]\nname = \"probe\"\n")
                .as_deref(),
            Some("probe"),
            "including a `[[bin]]` table's, wherever it sits"
        );
        assert_eq!(
            package_name("[package]\nversion = \"0.1.0\"\n\n[dependencies]\nname = \"other\"\n"),
            None,
            "a `name` key in a later table is not the package's either"
        );
        assert_eq!(
            package_name("[workspace]\nmembers = [\"crates/*\"]\n"),
            None,
            "a manifest with no `[package]` table names no package"
        );
    }

    #[test]
    fn a_byte_order_mark_does_not_blind_either_line_reader() {
        // A UTF-8 BOM sits inside line 1, so the header written there squeezes
        // to `\u{feff}[package]` and equals nothing either reader looks for.
        // Cargo reads such a manifest and builds it, so both blindnesses land on
        // a file that is fine: the name reader answered `None` and the scan went
        // red on it, and the bin reader missed a line-1 `[[bin]]` header — the
        // silent half, measured letting a real binary-producing member through
        // unclassified and unscanned.
        assert_eq!(
            package_name("\u{feff}[package]\nname = \"probe\"\n").as_deref(),
            Some("probe"),
            "the mark is off before the header comparison"
        );
        assert!(
            declares_bin_table("\u{feff}[[bin]]\nname = \"probe\"\n"),
            "and off before the bin header's, which is the same rule"
        );
    }

    #[test]
    fn three_shapes_cargo_accepts_answer_none_rather_than_a_wrong_name() {
        // The bound the test above does not claim: three shapes cargo accepts
        // that this line-shaped reader declines, the multi-line string in both
        // its spellings. Each is pinned so the limit is measured rather than
        // asserted — a fix that later closes one reddens its row, and the row
        // is what gets updated. All of them land on `workspace_packages`'s
        // failure, which names these shapes.
        assert_eq!(
            package_name("package.name = \"probe\"\n"),
            None,
            "a `[package]` table declared by dotted keys is not a `[package]` line"
        );
        assert_eq!(
            package_name("[package]\nname = \"\"\"probe\"\"\"\n"),
            None,
            "a multi-line string is declined, not misread as the empty name"
        );
        assert_eq!(
            package_name("[package]\nname = '''probe'''\n"),
            None,
            "and its literal spelling, which cargo reads as `probe` too"
        );
        assert_eq!(
            package_name("[package]\nkeywords = [\n\"[db]\"\n]\nname = \"probe\"\n"),
            None,
            "a quoted array element squeezes to a table header's shape and ends `[package]`"
        );
        // How narrow that last one is: the element has to sit alone on a line that
        // does not end in a comma (a trailing comma leaves the squeezed line ending
        // in `,`). Being last in the array is neither necessary — a leading-comma
        // style puts a bare element mid-array — nor sufficient — a closer on the
        // element's line is still a header shape.
        assert_eq!(
            package_name("[package]\nkeywords = [\n\"[db]\",\n]\nname = \"probe\"\n").as_deref(),
            Some("probe"),
            "the same element followed by a comma does not read as a header"
        );
    }

    #[test]
    fn a_name_inside_a_multi_line_string_in_package_is_read_instead_of_the_real_one() {
        // The shape that defeats the table rule rather than the line rules: the
        // decoy sits inside `[package]`, in a multi-line string value, so no
        // header tracking reaches it. Cargo reads `realname` (measured, `cargo
        // metadata`) and this reader reads `evil`. For a workspace member that
        // is loud rather than silent — the scan and the walk's roots then
        // disagree under two names, see `member_set_gaps` — and it is pinned
        // here so that closing it reddens this row.
        assert_eq!(
            package_name(
                "[package]\ndescription = \"\"\"\nname = \"evil\"\n\"\"\"\nname = \"realname\"\n"
            )
            .as_deref(),
            Some("evil"),
            "this reader has no notion of a multi-line string value"
        );
    }

    #[test]
    fn the_two_scan_failures_carry_different_exit_codes() {
        // The mapping table on its own, both directions: a manifest whose name
        // this reader cannot find is input the check refuses (`EX_DATAERR`),
        // where the filesystem refusing to be read is the tool unable to work
        // (`EX_UNAVAILABLE`). The first direction also has a test that reaches
        // it through a real scan (below); the second has none.
        assert_eq!(
            ScanFailure::NoPackageName(String::new()).exit_code(),
            EXIT_POLICY_VIOLATION
        );
        assert_eq!(
            ScanFailure::Unreadable(String::new()).exit_code(),
            EXIT_TOOLING_FAILURE
        );
    }

    #[test]
    fn a_manifest_the_reader_cannot_read_takes_the_whole_audit_to_a_policy_failure() {
        // Three hops over a synthetic `crates/`, which is what makes this a
        // guard rather than a restatement: the scan turns a name it cannot find
        // into `NoPackageName` (not `Unreadable`), that variant carries
        // `EX_DATAERR`, and `audit` returns the code the failure carries
        // instead of one of its own. The name is written in a shape cargo
        // accepts — measured, `cargo metadata` reads `name = """probe"""` as
        // the package `probe` — so what is pinned is this reader's limit, not
        // a broken file.
        let layout = CrateLayout::new("scan_failure");
        layout.file_with(
            "crates/probe/Cargo.toml",
            "[package]\nname = \"\"\"probe\"\"\"\nversion = \"0.1.0\"\n",
        );

        let Err(failure) = workspace_packages(&layout.dir) else {
            panic!("a name written as a multi-line string is one this reader declines");
        };
        assert_eq!(
            failure.exit_code(),
            EXIT_POLICY_VIOLATION,
            "an unreadable name is policy-shaped input, not a tooling failure: {failure}"
        );
        let message = failure.to_string();
        let manifest = layout.dir.join("crates/probe/Cargo.toml");
        assert!(
            message.contains(&manifest.display().to_string())
                && message.contains("multi-line string"),
            "the failure names the file and the shapes measured to defeat the reader: {message}"
        );
        assert_eq!(
            audit(&layout.dir),
            EXIT_POLICY_VIOLATION,
            "and the audit forwards that code rather than substituting one of its own"
        );
        assert_eq!(
            audit(&layout.dir.join("nowhere")),
            EXIT_TOOLING_FAILURE,
            "while a root whose `crates/` cannot be read is the tooling failure it was"
        );
    }

    #[test]
    fn a_workspace_walk_splits_into_one_closure_per_root() {
        // What `cargo tree --workspace` emits: consecutive depth-0 rows, each
        // starting a new tree. Splitting on anything else (say, a blank line)
        // would merge two members' closures and report a violation under the
        // wrong root.
        let trees = split_trees(&nodes(&[
            (0, "alpha"),
            (1, "shared"),
            (0, "beta"),
            (1, "shared"),
            (2, "pglet_testing"),
        ]));
        assert_eq!(trees.len(), 2, "two roots: {:?}", trees.len());
        assert_eq!(trees[0].0, "alpha");
        assert_eq!(trees[0].1.len(), 2, "alpha's closure is its own two rows");
        assert!(violations("alpha", &trees[0].1).is_empty());
        assert_eq!(trees[1].0, "beta");
        assert_eq!(violations("beta", &trees[1].1).len(), 1, "beta reaches it");
    }

    #[test]
    fn a_depth_row_is_parsed_and_a_non_depth_row_is_dropped() {
        let parsed = parse_nodes("0|alpha v0.1.0 (/x)\n\n1|serde v1.0.0\nnote: something\n")
            .expect("parses");
        assert_eq!(parsed.len(), 2, "two nodes: {}", parsed.len());
        assert_eq!((parsed[0].depth, parsed[0].name.as_str()), (0, "alpha"));
        assert_eq!((parsed[1].depth, parsed[1].name.as_str()), (1, "serde"));
    }

    #[test]
    fn a_digit_initial_package_name_does_not_bleed_into_the_depth() {
        // Without the `|` the depth is flush against the package, so this row
        // reads as depth 13 of a package `des` — a wrong ancestor chain, and a
        // missed violation if such a package were the forbidden crate's parent.
        let parsed = parse_nodes("1|3des v0.2.0\n").expect("parses");
        assert_eq!(parsed.len(), 1, "one node: {}", parsed.len());
        assert_eq!((parsed[0].depth, parsed[0].name.as_str()), (1, "3des"));
    }

    #[test]
    fn the_scanned_packages_and_the_walk_roots_must_be_the_same_set() {
        let package = |name: &str| WorkspacePackage {
            name: name.to_owned(),
            produces_artifact: false,
        };
        let tree = |name: &str| (name.to_owned(), Vec::new());

        assert!(
            member_set_gaps(
                &[package("alpha"), package("beta")],
                &[tree("alpha"), tree("beta")]
            )
            .is_empty(),
            "the sets agree"
        );

        // A member declared outside `crates/`: walked, never classified.
        let unscanned = member_set_gaps(&[package("alpha")], &[tree("alpha"), tree("tool")]);
        assert_eq!(unscanned.len(), 1, "one gap: {unscanned:?}");
        assert!(
            unscanned[0].contains("`tool`")
                && unscanned[0].contains("scan produced no package of that name"),
            "it names the unscanned member and states only what it compared: {}",
            unscanned[0]
        );
        assert!(
            unscanned[0].contains("not *directly* under `crates/`")
                && unscanned[0].contains("package_name"),
            "and lists both causes, the misread name included: {}",
            unscanned[0]
        );

        // A manifest under `crates/` cargo does not treat as a member.
        let unwalked = member_set_gaps(&[package("alpha"), package("ghost")], &[tree("alpha")]);
        assert_eq!(unwalked.len(), 1, "one gap: {unwalked:?}");
        assert!(
            unwalked[0].contains("`ghost`")
                && unwalked[0].contains("exclude")
                && unwalked[0].contains("package_name"),
            "it names the unwalked package and the causes it can name: {}",
            unwalked[0]
        );
    }

    #[test]
    fn an_explicit_bin_table_produces_an_artifact() {
        // Every one of these builds a real binary, and none of them leaves an
        // auto-discoverable file for the layout arms to find: the `path` points
        // outside `src/`, so the spelling is the only thing standing between the
        // crate and an unscanned ship.
        let layout = CrateLayout::new("bin_table");
        layout.file("src/lib.rs");
        let manifest =
            "[package]\nname = \"probe\"\n\n[[bin]]\nname = \"probe\"\npath = \"other/place.rs\"\n";
        assert!(
            produces_artifact(manifest, &layout.dir),
            "a `[[bin]]` table is the signal, wherever its `path` points"
        );
        assert!(
            declares_bin_table("[package]\nname = \"p\"\n[[ bin ]]\n"),
            "whitespace inside the brackets is legal TOML"
        );
        assert!(
            declares_bin_table("[package]\nname = \"p\"\n[[bin]]  # the shipped probe\n"),
            "a trailing comment on the header is legal TOML"
        );
        assert!(
            declares_bin_table(
                "bin = [{ name = \"p\", path = \"other/place.rs\" }]\n\n[package]\nname = \"p\"\n"
            ),
            "the top-level inline array is the same table with no header"
        );
        assert!(
            declares_bin_table("bin = [\n  { name = \"p\" },\n]\n\n[package]\nname = \"p\"\n"),
            "a multi-line inline array matches on its first line"
        );
        // The quoted-key spellings. TOML writes a bare key and a quoted key
        // interchangeably; cargo builds a real binary from each of these
        // (measured, `cargo metadata` and `cargo build`), and each is invisible
        // to a reader that compares before squeezing the quotes out.
        assert!(
            declares_bin_table("[package]\nname = \"p\"\n[[\"bin\"]]\n"),
            "a basic-string key in the header position"
        );
        assert!(
            declares_bin_table("[package]\nname = \"p\"\n[['bin']]\n"),
            "a literal-string key in the header position"
        );
        assert!(
            declares_bin_table("[package]\nname = \"p\"\n[[ \"bin\" ]]  # the probe\n"),
            "with the whitespace and the comment the bare form already tolerated"
        );
        assert!(
            declares_bin_table(
                "\"bin\" = [{ name = \"p\", path = \"other/place.rs\" }]\n\n[package]\nname = \"p\"\n"
            ),
            "a basic-string key in the inline position"
        );
        assert!(
            declares_bin_table(
                "'bin' = [{ name = \"p\", path = \"other/place.rs\" }]\n\n[package]\nname = \"p\"\n"
            ),
            "a literal-string key in the inline position"
        );
        assert!(
            declares_bin_table("[package.metadata]\nbin = \"[str]\"\n"),
            "the over-match the doc names: the quotes come out before the `bin=[` test, so a \
             string value beginning with `[` matches too"
        );
        assert!(
            !declares_bin_table("[package]\nname = \"p\"\n# [[bin]]\n[[bench]]\n"),
            "a commented-out header and another array of tables are not a bin table"
        );
        assert!(
            !declares_bin_table("[package]\nname = \"p\"\n[[binary]]\n"),
            "squeezing the quotes must not widen the header arm past the key `bin`"
        );
    }

    #[test]
    fn src_main_rs_produces_an_artifact() {
        let layout = CrateLayout::new("src_main");
        layout.file("src/main.rs");
        assert!(produces_artifact(LIB_ONLY, &layout.dir));
    }

    #[test]
    fn a_flat_file_in_src_bin_produces_an_artifact() {
        let layout = CrateLayout::new("bin_flat");
        layout.file("src/lib.rs").file("src/bin/probe.rs");
        assert!(produces_artifact(LIB_ONLY, &layout.dir));
    }

    #[test]
    fn a_subdirectory_of_src_bin_with_a_main_rs_produces_an_artifact() {
        // Cargo's fourth auto-discovery shape. A scan that only tests file
        // extensions sees a directory with no `.rs` extension and reports no
        // binary, so the package goes unclassified and unscanned — the exact
        // failure the forced classification exists to prevent.
        let layout = CrateLayout::new("bin_dir");
        layout.file("src/lib.rs").file("src/bin/probe/main.rs");
        assert!(produces_artifact(LIB_ONLY, &layout.dir));
    }

    #[test]
    fn a_subdirectory_of_src_bin_without_a_main_rs_does_not() {
        let layout = CrateLayout::new("bin_dir_no_main");
        layout.file("src/lib.rs").file("src/bin/probe/helper.rs");
        assert!(!produces_artifact(LIB_ONLY, &layout.dir));
    }

    #[test]
    fn a_cdylib_produces_an_artifact_and_a_plain_lib_does_not() {
        let layout = CrateLayout::new("crate_type");
        layout.file("src/lib.rs");
        assert!(produces_artifact(
            "[package]\nname = \"probe\"\n\n[lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n",
            &layout.dir
        ));
        assert!(produces_artifact(
            "[package]\nname = \"probe\"\n\n[lib]\ncrate-type = [\n  \"staticlib\",\n]\n",
            &layout.dir
        ));
        assert!(!produces_artifact(
            "[package]\nname = \"probe\"\n\n[lib]\ncrate-type = [\"rlib\"]\n",
            &layout.dir
        ));
        assert!(!produces_artifact(LIB_ONLY, &layout.dir));
    }

    #[test]
    fn every_artifact_package_on_disk_is_classified() {
        // The completeness check run against the real workspace: this is the
        // test that fails when a new binary or cdylib crate lands unclassified.
        let packages = workspace_packages(&repo_root()).expect("read crates/");
        assert!(
            packages.iter().any(|p| p.produces_artifact),
            "the scan found no artifact-producing package at all, so it proves nothing"
        );
        let gaps = classification_gaps(&packages);
        assert!(gaps.is_empty(), "{gaps:#?}");
    }

    #[test]
    fn the_wasm_cdylib_is_among_the_scanned_artifacts() {
        // `pglet_wasm` produces no binary target, so only the `crate-type` arm
        // puts it in front of the classification check. Losing that arm would
        // leave the npm artifact silently unscanned rather than loudly
        // unclassified.
        let packages = workspace_packages(&repo_root()).expect("read crates/");
        let wasm = packages
            .iter()
            .find(|p| p.name == "pglet_wasm")
            .expect("pglet_wasm is a workspace member");
        assert!(wasm.produces_artifact, "the cdylib counts as an artifact");
        assert!(
            PRODUCTION_ARTIFACT_PACKAGES.contains(&"pglet_wasm"),
            "and it is scanned as one"
        );
    }
}
