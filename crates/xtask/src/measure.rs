//! `cargo xtask measure` — the quiet-box measurement set behind the README's figures.
//!
//! The counterpart to [`crate::gates`], and the split is the point: the gate set
//! answers *is the engine correct* and runs anywhere, while this answers *what
//! are the numbers* and is only meaningful on an otherwise-idle machine with a
//! live Postgres and a fuz_app checkout (`PGLET_FUZ_APP_DIR`, default `../fuz_app`).
//! They share their plumbing ([`crate::runner`]) and nothing else.
//!
//! Its Postgres address is `127.0.0.1:5432` as the `postgres` superuser, written
//! here rather than read from the harness's `PGLET_PG_URL` knob — this step
//! *creates and drops* its own scratch database, which is an administrative
//! connection rather than the oracle, and `xtask` depends on `pglet_core` alone,
//! so reading that knob here would mean a second copy of its parse rather than
//! one reader of one rule.
//!
//! ```sh
//! cargo xtask measure                        # the whole set
//! cargo xtask measure --skip exit-proof      # ...minus a step
//! cargo xtask measure --only startup,gates   # ...or just these
//! cargo xtask measure --reps 3 --legs pglite,pglet-wasm,pg
//! cargo xtask measure --dry-run             # ...print the plan, run nothing
//! ```
//!
//! `--dry-run` earns its place: **every bench** step here refreshes a recorded
//! results JSON, so no bench step can be run "just to see whether the runner
//! works" — trying one overwrites a real measurement with whatever the box was
//! doing at the time. (`--only gates` is the other safe probe: it writes no
//! bench results.)
//!
//! # Why the order
//!
//! Quiet-sensitive first. Measured on this box, the startup bench read 5–6 ms p50
//! at 40–60% CV under a concurrent wasm build against ≈1 ms at under 15% CV
//! with the box idle — so the steps most easily spoiled run while the machine is
//! most likely still quiet, and the steps that do not care run last: `size-compare`
//! (byte counts, not timings — though its `build:npm` loads the box for what follows)
//! and then `gates --check`.
//!
//! Ratios *within* one run survive a noisy box, because every leg pays the same
//! tax; absolute figures and cross-run comparisons do not. That is why the
//! idle check below warns rather than refuses — it reads the 1-minute load and a
//! CPU sample over a couple of seconds before the first step, which says whether
//! the box is busy *now* and nothing about the minutes the run takes — and why the
//! summary is worth reading before any figure is copied anywhere.
//!
//! # The figures
//!
//! The run ends with the figures list (`FIGURES`): every figure the README, the npm
//! README and `TRADEOFFS.md` quote from a file one of these steps writes — the exit-proof
//! multiples, the size comparison, the startup, wasm-init and head-to-head cold-init
//! latencies, and the pglite version behind each pglite figure: the exit-proof multiples ran
//! fuz_app's install, while the size comparison weighed and the head-to-head cold inits ran
//! `benches/`' (the lockfile's, which the preflight checks) — each with the file and key it
//! was read from.
//! The multiples and size ratios are computed by the benches that measured them —
//! under `ratios` in `exit_proof_results.json`, in `pglite_size_comparison.json` — never
//! divided by hand afterwards. A figure whose step did not run, or whose file the step did
//! not rewrite — or, in `wasm_compare_results.json`, whose runtime's slice it did not —
//! is listed as not refreshed rather than with the value a previous run recorded, and a
//! `null` in a fresh file is listed with why: the `null_reason` its bench wrote beside it,
//! else what a null in that slot means (`Figure::null_means`). The files are read with
//! `pglet_core`'s own JSON parser, the one dependency this crate has.
//!
//! Not listed, because no step writes them: the native binary's size (`size_baseline.json`,
//! which `size:update` writes and the gates step only checks against), the figures of
//! benches this set does not run (`bench:commit`, `bench:attribute`, `bench:index-accel`),
//! and the one-off measurements `TRADEOFFS.md` records beside its decisions.
//!
//! Every results file is stamped with the commit it measured, `-dirty` when the tree
//! differs from it and `-dirty?` when that could not be read (`benches/lib/results.ts`'s
//! `git_commit`, which the preflight asks too, so a dirty tree is named before the run
//! rather than discovered in the stamps). The preflight likewise asks
//! `benches/lib/pglite_install.ts` whether `benches/` holds the lockfile's pglite before
//! any step that loads it.
//!
//! # The pg leg's prerequisite
//!
//! `PGLET_EXITPROOF_PG_URL`'s database is dropped and recreated before every
//! pass, which is *not* what makes the pg leg work. `create_pg_factory` hands
//! every test file one persistent database and its `create()` drops only
//! `schema_version`, so under vitest's `isolate: false` an earlier file's DDL is
//! still standing when the next file's suite runs — and fuz_app's whole-schema
//! assertions then read tables their own suite never created. Only a
//! per-`create()` schema reset fixes that and it lives in fuz_app's fixtures.
//! The preflight below checks for it, because the alternative is discovering it
//! mid-run — which happened four times back when the run took twenty minutes.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, SystemTime};

use pglet_core::{JsonbValue, parse_jsonb};

use crate::runner::{TaskRun, deno_task, lock_run_dir, parse_name_list, repo_root, run_logged};
use Seg::{Item, Key};

/// One step of the set, in run order.
struct Step {
    /// CLI name, log file stem, and summary label.
    name: &'static str,
    /// What it produces, for the banner.
    title: &'static str,
}

const STEPS: &[Step] = &[
    Step {
        name: "exit-proof",
        title: "four-leg fuz_app `db` suite (pglite, pglet-wasm, pglet-server, pg) — ~3 min",
    },
    Step {
        name: "wasm-compare",
        title: "head-to-head cold-init vs pglite and sqlite-wasm (Deno/Node/Bun)",
    },
    Step {
        name: "startup",
        title: "native spawn→ready latency",
    },
    Step {
        name: "wasm-init",
        title: "wasm compile/instantiate/first-query latency",
    },
    Step {
        name: "size-compare",
        title: "pglet-vs-pglite payload bytes, raw and gzip (the shipped npm wasm)",
    },
    Step {
        name: "gates",
        title: "cargo xtask gates --check — the gate set's validation of this tree",
    },
];

/// Entry point for `cargo xtask measure`.
pub fn run(args: &[String]) -> ExitCode {
    let opts = match Opts::parse(args) {
        Ok(opts) => opts,
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    let root = repo_root();
    let selected: Vec<&Step> = STEPS.iter().filter(|s| opts.wants(s.name)).collect();
    if selected.is_empty() {
        eprintln!("error: every step was filtered out");
        return ExitCode::FAILURE;
    }

    if let Err(message) = preflight(&root, &selected) {
        eprintln!("error: {message}");
        return ExitCode::FAILURE;
    }

    if opts.dry_run {
        println!("\n=== plan (--dry-run; nothing is run) ===");
        for step in &selected {
            let cmd = command_for(step.name, &opts, &root);
            println!("{:<14} {:?}", step.name, cmd);
        }
        return ExitCode::SUCCESS;
    }

    let run_dir = match lock_run_dir(&root, "release_measurements", "measurement run") {
        Ok(dir) => dir,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };

    let mut outcomes: Vec<StepRun> = Vec::with_capacity(selected.len());
    for step in &selected {
        eprintln!("\n== {} — {} ==", step.name, step.title);
        let started = SystemTime::now();
        let log = run_dir.log_dir.join(format!("{}.log", step.name));
        let mut cmd = command_for(step.name, &opts, &root);
        // Streamed: a measurement's tables are the product, and a step that
        // prints nothing for minutes at a stretch cannot be watched or
        // interrupted on evidence. The log gets the same bytes.
        let outcome = run_logged(&mut cmd, &root, log, true);
        eprintln!(
            "\n[measure] {} {} ({:.0}s)",
            step.name,
            outcome.status(),
            outcome.secs
        );
        outcomes.push(StepRun {
            step,
            started,
            run: outcome,
        });
    }

    teardown(&opts, &outcomes);
    print_summary(&outcomes);
    print_leftover_scratch_dbs();
    print_figures(&root, &outcomes);

    let failed: Vec<&str> = outcomes
        .iter()
        .filter(|o| !o.run.ok())
        .map(|o| o.step.name)
        .collect();
    if failed.is_empty() {
        println!(
            "\nnext: update the README, npm README and TRADEOFFS.md figures from the list above,\n\
             then commit the refreshed benches/*.json"
        );
        ExitCode::SUCCESS
    } else {
        println!(
            "\n{} step(s) failed ({}) — read the logs above before trusting any figure",
            failed.len(),
            failed.join(", ")
        );
        ExitCode::FAILURE
    }
}

/// One step as it ran: its outcome and when it started, which dates the files it wrote.
struct StepRun<'a> {
    step: &'a Step,
    started: SystemTime,
    run: TaskRun,
}

/// The command one step runs.
fn command_for(name: &str, opts: &Opts, root: &Path) -> Command {
    match name {
        "exit-proof" => {
            let mut cmd = deno_task("bench:exit-proof:update");
            cmd.env(
                "PGLET_EXITPROOF_PG_URL",
                format!("postgres://postgres@127.0.0.1:5432/{}", opts.scratch_db),
            );
            cmd.env("PGLET_EXITPROOF_REPS", opts.reps.to_string());
            if let Some(legs) = &opts.legs {
                cmd.env("PGLET_EXITPROOF_LEGS", legs.join(","));
            }
            cmd
        }
        "wasm-compare" => deno_task("bench:wasm-compare:update"),
        "startup" => deno_task("bench:startup:update"),
        "wasm-init" => deno_task("bench:wasm-init:update"),
        "size-compare" => deno_task("bench:size-compare:update"),
        // Re-invoking this binary rather than `cargo xtask gates` keeps the step
        // free of a rebuild in the middle of a measurement run, and keeps its
        // own `target/gates` lock and logs exactly where a hand-run leaves them.
        _ => {
            let exe = std::env::current_exe().unwrap_or_else(|_| root.join("target/debug/xtask"));
            let mut cmd = Command::new(exe);
            cmd.args(["gates", "--check"]);
            cmd
        }
    }
}

/// Everything checkable in a second, checked before steps that take minutes.
fn preflight(root: &Path, selected: &[&Step]) -> Result<(), String> {
    let wants = |name: &str| selected.iter().any(|s| s.name == name);
    println!("== preflight ==");

    if wants("exit-proof") {
        if !ok(Command::new("psql").args([
            "-h",
            "127.0.0.1",
            "-p",
            "5432",
            "-U",
            "postgres",
            "-Atc",
            "select 1",
        ])) {
            return Err("no Postgres on 127.0.0.1:5432".to_owned());
        }
        // The checkout `bench:exit-proof` runs against: `PGLET_FUZ_APP_DIR`, as that bench
        // reads it, else `../fuz_app`.
        let fuz_app = std::env::var_os("PGLET_FUZ_APP_DIR")
            .map_or_else(|| root.join("../fuz_app"), PathBuf::from);
        if !fuz_app.exists() {
            return Err(format!(
                "{} not found (set PGLET_FUZ_APP_DIR to the fuz_app checkout)",
                fuz_app.display()
            ));
        }
        // A warning and not an error: the fixture could reasonably be spelled
        // another way, and a missing reset fails the whole run late. The suites
        // import `db_fixture.ts`, which builds its factories through
        // `create_db_fixture.ts`; the reset may sit in either.
        let fixtures = [
            fuz_app.join("src/test/db_fixture.ts"),
            fuz_app.join("src/test/create_db_fixture.ts"),
        ];
        let reset = fixtures.iter().any(|fixture| {
            std::fs::read_to_string(fixture).is_ok_and(|text| text.contains("drop_auth_schema"))
        });
        if !reset {
            println!(
                "  ⚠ neither {} nor {} references drop_auth_schema — the pg leg will fail\n    \
                 on fuz_app's whole-schema assertions (see this module's docs)",
                fixtures[0].display(),
                fixtures[1].display()
            );
        }
    }
    // Every step but startup builds the wasm artifact somewhere in its chain
    // (exit-proof, wasm-compare and size-compare through `build:npm`, `gates` through
    // `size:check`), so this is nearly unconditional.
    if (wants("exit-proof")
        || wants("wasm-compare")
        || wants("wasm-init")
        || wants("size-compare")
        || wants("gates"))
        && !on_path("wasm-pack")
    {
        return Err("wasm-pack not on PATH".to_owned());
    }
    if wants("wasm-compare") {
        for tool in ["node", "bun"] {
            if !on_path(tool) {
                return Err(format!("{tool} not on PATH (needed by wasm-compare)"));
            }
        }
    }
    // Both load pglite from `benches/` and both refuse one that is not the lockfile's
    // version; asked here of the same rule (`benches/lib/pglite_install.ts`), so a wrong
    // install is refused before the first timed step rather than by size-compare after
    // the last of them.
    if wants("wasm-compare") || wants("size-compare") {
        let version = deno_eval(
            root,
            "import {assert_pglite_matches_lockfile} from './benches/lib/pglite_install.ts'; \
             try { console.log(assert_pglite_matches_lockfile().version); } \
             catch (e) { console.error(e instanceof Error ? e.message : String(e)); Deno.exit(1); }",
        )?;
        println!("pglite (benches/): {version}, the lockfile's");
    }
    if wants("size-compare") && !on_path("gzip") {
        return Err("gzip not on PATH (needed by size-compare)".to_owned());
    }

    // Every step but gates stamps the results file it writes.
    if selected.iter().any(|s| s.name != "gates") {
        match tree_stamp(root) {
            Some(stamp) if stamp.ends_with("-dirty") || stamp.ends_with("-dirty?") => println!(
                "stamp: {stamp}\n  ⚠ the tree differs from HEAD — every results file this run writes \
                 will be stamped `{stamp}`;\n    commit (or stash) first for a stamp that names \
                 the measured tree"
            ),
            Some(stamp) => println!("stamp: {stamp}"),
            None => println!("stamp: unreadable (`deno eval` of benches/lib/results.ts failed)"),
        }
    }

    print_idle_check();
    Ok(())
}

/// The stamp the benches will write, asked of the one rule that writes it
/// (`benches/lib/results.ts`'s `git_commit`) rather than a second copy of it here.
fn tree_stamp(root: &Path) -> Option<String> {
    deno_eval(
        root,
        "import {git_commit} from './benches/lib/results.ts'; console.log(git_commit());",
    )
    .ok()
    .filter(|stamp| !stamp.is_empty())
}

/// Run `code` under `deno eval` from the repo root — how the preflight asks a bench's own
/// rule rather than copying it here. Its trimmed stdout, or, when it fails, its trimmed
/// stderr.
fn deno_eval(root: &Path, code: &str) -> Result<String, String> {
    let out = Command::new("deno")
        .args(["eval", code])
        .current_dir(root)
        .env_remove("LD_LIBRARY_PATH")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("`deno eval` could not run: {e}"))?;
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_owned();
    if out.status.success() {
        Ok(text(&out.stdout))
    } else {
        Err(text(&out.stderr))
    }
}

/// How long the CPU is sampled for the idle check.
const CPU_SAMPLE: Duration = Duration::from_secs(2);

/// Print the idle check: the 1-minute load and a CPU-busy sample, each against its
/// threshold. A warning, never a refusal (see this module's docs).
fn print_idle_check() {
    let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let load1 = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok());
    let before = std::fs::read_to_string("/proc/stat").ok();
    std::thread::sleep(CPU_SAMPLE);
    let after = std::fs::read_to_string("/proc/stat").ok();
    let busy = before
        .zip(after)
        .and_then(|(before, after)| cpu_busy_fraction(&before, &after));

    println!(
        "idle check ({cpus} CPUs) — this box should be otherwise idle:\n  \
         1-min load {}  (warns above {:.1})\n  \
         CPU busy   {}  (warns above {:.0}%, over {}s)",
        load1.map_or_else(|| "unreadable".to_owned(), |l| format!("{l:.2}")),
        load_threshold(cpus),
        busy.map_or_else(|| "unreadable".to_owned(), |b| format!("{:.1}%", b * 100.0)),
        BUSY_THRESHOLD * 100.0,
        CPU_SAMPLE.as_secs()
    );
    for warning in idle_warnings(load1, busy, cpus) {
        println!("  ⚠ {warning}");
    }
}

/// The CPU-busy fraction above which the idle check warns: a tenth of the machine — on
/// any box, more than a background trickle, and well under one build.
const BUSY_THRESHOLD: f64 = 0.10;

/// The 1-minute load above which the idle check warns: a tenth of the CPUs, but never
/// under one runnable task, which a quiet machine of any size is not.
fn load_threshold(cpus: usize) -> f64 {
    (cpus as f64 * 0.1).max(1.0)
}

/// The idle check's warnings over its readings; empty on a quiet box.
fn idle_warnings(load1: Option<f64>, busy: Option<f64>, cpus: usize) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(busy) = busy.filter(|b| *b > BUSY_THRESHOLD) {
        warnings.push(format!(
            "the CPUs are {:.0}% busy right now — close other work, or expect the absolute \
             figures to flap",
            busy * 100.0
        ));
    }
    if let Some(load) = load1.filter(|l| *l > load_threshold(cpus)) {
        let now = if busy.is_some_and(|b| b <= BUSY_THRESHOLD) {
            " (the CPU sample is quiet, so this is likely work that just finished — a build \
             still weighs on the 1-minute load for a minute or two)"
        } else {
            ""
        };
        warnings.push(format!("the 1-minute load is {load:.2}{now}"));
    }
    warnings
}

/// The fraction of all CPU time spent busy between two `/proc/stat` readings, from
/// their aggregate `cpu` lines: busy is everything but `idle` and `iowait`.
fn cpu_busy_fraction(before: &str, after: &str) -> Option<f64> {
    let (busy_before, total_before) = cpu_times(before)?;
    let (busy_after, total_after) = cpu_times(after)?;
    let total = total_after.checked_sub(total_before)?;
    let busy = busy_after.checked_sub(busy_before)?;
    (total > 0).then(|| busy as f64 / total as f64)
}

/// `(busy, total)` jiffies from `/proc/stat`'s aggregate `cpu` line: its first eight
/// fields — user, nice, system, idle, iowait, irq, softirq, steal — as `guest` and
/// `guest_nice` are already counted in `user` and `nice`.
fn cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if fields.len() < 5 {
        return None;
    }
    let total: u64 = fields.iter().sum();
    let idle = fields[3] + fields[4];
    Some((total - idle, total))
}

/// Drop the scratch database, unless it is worth keeping.
///
/// It is disposable by construction — the exit-proof bench drops and recreates
/// it before every pass — so the only time its contents are worth anything is
/// when that step failed and left the state the failure happened in.
fn teardown(opts: &Opts, outcomes: &[StepRun]) {
    let exit_proof_ok = outcomes
        .iter()
        .any(|o| o.step.name == "exit-proof" && o.run.ok());
    if !exit_proof_ok || opts.keep_scratch {
        return;
    }
    let dropped = ok(Command::new("dropdb").args([
        "-h",
        "127.0.0.1",
        "-p",
        "5432",
        "-U",
        "postgres",
        "--if-exists",
        &opts.scratch_db,
    ]));
    if dropped {
        println!("\ndropped scratch db {}", opts.scratch_db);
    }
}

fn print_summary(outcomes: &[StepRun]) {
    println!("\n=== release measurements ===");
    for o in outcomes {
        println!(
            "{:<14} {:>14} {:>7.0}s  {}",
            o.step.name,
            o.run.status(),
            o.run.secs,
            o.run.log.display()
        );
    }
}

/// Name the scratch databases lying around, rather than hardcoding a list.
///
/// A hardcoded list reads as "these exist" long after they were dropped.
fn print_leftover_scratch_dbs() {
    let out = Command::new("psql")
        .args([
            "-h",
            "127.0.0.1",
            "-p",
            "5432",
            "-U",
            "postgres",
            "-Atc",
            "select datname from pg_database where datname ~ '(exitproof|pglet.*scratch|scratch.*pglet)'",
        ])
        .output();
    let Ok(out) = out else { return };
    let names: Vec<&str> = std::str::from_utf8(&out.stdout)
        .unwrap_or("")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if names.is_empty() {
        return;
    }
    println!("\nleftover scratch dbs you may want to drop:");
    for name in names {
        println!("  dropdb -h 127.0.0.1 -p 5432 -U postgres {name}");
    }
}

/// How a figure's value prints.
#[derive(Clone, Copy, Debug)]
enum Unit {
    /// A ratio.
    Times,
    /// A duration in milliseconds.
    Millis,
    /// A byte count, in KB.
    Kilobytes,
    /// A byte count, in MB.
    Megabytes,
    /// A string, as recorded.
    Text,
}

/// One step of a key path into a results file.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Seg {
    /// An object's member.
    Key(&'static str),
    /// The element of an array whose string member `field` is `value` — a scenario or an
    /// engine row, which the benches record as arrays.
    Item {
        field: &'static str,
        value: &'static str,
    },
}

/// The `Item` step selecting the element whose `field` is `value`.
const fn item(field: &'static str, value: &'static str) -> Seg {
    Item { field, value }
}

/// One figure the README, the npm README or `TRADEOFFS.md` quotes, and where a run records it.
#[derive(Debug)]
struct Figure {
    /// Where it is quoted and what it is — never its value, which is what this prints.
    quoted: &'static str,
    /// The step that writes it.
    step: &'static str,
    /// The results file, relative to the repo root.
    file: &'static str,
    /// The key path to it within that file.
    path: &'static [Seg],
    /// The object within the file whose `timestamp` and `git_commit` date and stamp the
    /// figure: the whole document (`&[]`), or a slice of it one step's run rewrites alone —
    /// wasm-compare's per-runtime slices, written by three runs into one file.
    envelope: &'static [Seg],
    unit: Unit,
    /// What a `null` in its slot means, where its bench records one; a `null_reason` the
    /// bench writes beside the value is printed in its place.
    null_means: Option<&'static str>,
}

/// What a `null` exit-proof ratio means, where the file holds no `null_reason` for it.
const RATIO_NULL: Option<&str> =
    Some("a leg failed or was not run, or the two legs passed different test counts");

const EXIT_PROOF_JSON: &str = "benches/exit_proof_results.json";
const SIZE_COMPARISON_JSON: &str = "benches/pglite_size_comparison.json";
const STARTUP_JSON: &str = "benches/startup_results.json";
const WASM_INIT_JSON: &str = "benches/wasm_init_results.json";
const WASM_COMPARE_JSON: &str = "benches/wasm_compare_results.json";

/// Every quoted figure a step of this run writes. The exit-proof keys are
/// `benches/lib/exit_proof_ratios.ts`'s `EXIT_PROOF_RATIOS` and `exit_proof.ts`'s `engines`;
/// the rest are the documents their benches build. `every_figure_resolves_in_its_committed_file`
/// holds each path to the file its bench last committed.
const FIGURES: &[Figure] = &[
    Figure {
        quoted: "README intro + npm README lead: faster than pglite (tests p50, pglite / pglet-wasm)",
        step: "exit-proof",
        file: EXIT_PROOF_JSON,
        path: &[
            Key("ratios"),
            Key("pglite_over_pglet_wasm_tests"),
            Key("value"),
        ],
        envelope: &[],
        unit: Unit::Times,
        null_means: RATIO_NULL,
    },
    Figure {
        quoted: "README intro: faster than a real PostgreSQL server (tests p50, pg / pglet-wasm — \
                 crosses tiers, quoted as in-process vs over TCP)",
        step: "exit-proof",
        file: EXIT_PROOF_JSON,
        path: &[Key("ratios"), Key("pg_over_pglet_wasm_tests"), Key("value")],
        envelope: &[],
        unit: Unit::Times,
        null_means: RATIO_NULL,
    },
    Figure {
        quoted: "README intro: server-to-server (tests p50, pg / pglet-server)",
        step: "exit-proof",
        file: EXIT_PROOF_JSON,
        path: &[
            Key("ratios"),
            Key("pg_over_pglet_server_tests"),
            Key("value"),
        ],
        envelope: &[],
        unit: Unit::Times,
        null_means: RATIO_NULL,
    },
    Figure {
        quoted: "README intro: vitest's whole run vs pglite (total p50, pglite / pglet-wasm)",
        step: "exit-proof",
        file: EXIT_PROOF_JSON,
        path: &[
            Key("ratios"),
            Key("pglite_over_pglet_wasm_total"),
            Key("value"),
        ],
        envelope: &[],
        unit: Unit::Times,
        null_means: RATIO_NULL,
    },
    Figure {
        quoted: "README intro + § Size and startup, npm README lead: the gzipped size, as a fraction of \
                 pglite's (pglite / pglet)",
        step: "size-compare",
        file: SIZE_COMPARISON_JSON,
        path: &[Key("ratios_pglite_over_pglet"), Key("gzip_total")],
        envelope: &[],
        unit: Unit::Times,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup: the raw payload, as a fraction of pglite's (pglite / pglet)",
        step: "size-compare",
        file: SIZE_COMPARISON_JSON,
        path: &[Key("ratios_pglite_over_pglet"), Key("raw_total")],
        envelope: &[],
        unit: Unit::Times,
        null_means: None,
    },
    Figure {
        quoted: "README intro (\"well under a megabyte over the wire\") + § Size and startup, npm README lead, \
                 TRADEOFFS.md's size table: pglet's wasm, gzipped",
        step: "size-compare",
        file: SIZE_COMPARISON_JSON,
        path: &[Key("pglet"), Key("wasm_gzip_bytes")],
        envelope: &[],
        unit: Unit::Kilobytes,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup, TRADEOFFS.md's size table: pglet's wasm, raw",
        step: "size-compare",
        file: SIZE_COMPARISON_JSON,
        path: &[Key("pglet"), Key("wasm_raw_bytes")],
        envelope: &[],
        unit: Unit::Kilobytes,
        null_means: None,
    },
    Figure {
        quoted: "npm README lead: pglite's wasm + data, gzipped",
        step: "size-compare",
        file: SIZE_COMPARISON_JSON,
        path: &[Key("pglite"), Key("total_gzip_bytes")],
        envelope: &[],
        unit: Unit::Megabytes,
        null_means: None,
    },
    Figure {
        quoted: "the pglite the exit-proof multiples ran (fuz_app's install)",
        step: "exit-proof",
        file: EXIT_PROOF_JSON,
        path: &[Key("engines"), Key("pglite")],
        envelope: &[],
        unit: Unit::Text,
        null_means: Some("the pglite leg was not selected (`--legs`, `PGLET_EXITPROOF_LEGS`)"),
    },
    Figure {
        quoted: "npm README lead: the pglite the size figures weighed and the head-to-head cold \
                 inits ran (benches/' install)",
        step: "size-compare",
        file: SIZE_COMPARISON_JSON,
        path: &[Key("pglite"), Key("version")],
        envelope: &[],
        unit: Unit::Text,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup, TRADEOFFS.md: native spawn→ready, ~1 ms p50 across storage modes \
                 — memory",
        step: "startup",
        file: STARTUP_JSON,
        path: &[Key("results"), item("scenario", "memory"), Key("p50")],
        envelope: &[],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup, TRADEOFFS.md: native spawn→ready — btree (in-mem)",
        step: "startup",
        file: STARTUP_JSON,
        path: &[
            Key("results"),
            item("scenario", "btree (in-mem)"),
            Key("p50"),
        ],
        envelope: &[],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup, TRADEOFFS.md: native spawn→ready — btree (file, fresh)",
        step: "startup",
        file: STARTUP_JSON,
        path: &[
            Key("results"),
            item("scenario", "btree (file, fresh)"),
            Key("p50"),
        ],
        envelope: &[],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup, TRADEOFFS.md: native spawn→ready — btree (file, reopen)",
        step: "startup",
        file: STARTUP_JSON,
        path: &[
            Key("results"),
            item("scenario", "btree (file, reopen)"),
            Key("p50"),
        ],
        envelope: &[],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup, TRADEOFFS.md: in-process wasm init, a full cold load (p50)",
        step: "wasm-init",
        file: WASM_INIT_JSON,
        path: &[
            Key("results"),
            item("scenario", "full cold load"),
            Key("p50"),
        ],
        envelope: &[],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "TRADEOFFS.md: wasm init, the first-ever compile (one shot)",
        step: "wasm-init",
        file: WASM_INIT_JSON,
        path: &[Key("compile_cold_one_shot_ms")],
        envelope: &[],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "TRADEOFFS.md: wasm init, instantiating from a cached module (p50)",
        step: "wasm-init",
        file: WASM_INIT_JSON,
        path: &[
            Key("results"),
            item("scenario", "cached instantiate"),
            Key("p50"),
        ],
        envelope: &[],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup, TRADEOFFS.md: pglet's head-to-head cold init — Deno",
        step: "wasm-compare",
        file: WASM_COMPARE_JSON,
        path: &[
            Key("runtimes"),
            Key("deno"),
            Key("results"),
            item("engine", "pglet"),
            Key("cold_init_ms"),
        ],
        envelope: &[Key("runtimes"), Key("deno")],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup, TRADEOFFS.md: pglet's head-to-head cold init — Node",
        step: "wasm-compare",
        file: WASM_COMPARE_JSON,
        path: &[
            Key("runtimes"),
            Key("node"),
            Key("results"),
            item("engine", "pglet"),
            Key("cold_init_ms"),
        ],
        envelope: &[Key("runtimes"), Key("node")],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "README § Size and startup, TRADEOFFS.md: pglet's head-to-head cold init — Bun",
        step: "wasm-compare",
        file: WASM_COMPARE_JSON,
        path: &[
            Key("runtimes"),
            Key("bun"),
            Key("results"),
            item("engine", "pglet"),
            Key("cold_init_ms"),
        ],
        envelope: &[Key("runtimes"), Key("bun")],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "README intro (\"pglite's second-plus init\") + § What it's for (\"pglite takes \
                 over a second\") + § Size and startup, TRADEOFFS.md: pglite's head-to-head cold init — Deno",
        step: "wasm-compare",
        file: WASM_COMPARE_JSON,
        path: &[
            Key("runtimes"),
            Key("deno"),
            Key("results"),
            item("engine", "pglite"),
            Key("cold_init_ms"),
        ],
        envelope: &[Key("runtimes"), Key("deno")],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "the same, pglite's head-to-head cold init — Node",
        step: "wasm-compare",
        file: WASM_COMPARE_JSON,
        path: &[
            Key("runtimes"),
            Key("node"),
            Key("results"),
            item("engine", "pglite"),
            Key("cold_init_ms"),
        ],
        envelope: &[Key("runtimes"), Key("node")],
        unit: Unit::Millis,
        null_means: None,
    },
    Figure {
        quoted: "the same, pglite's head-to-head cold init — Bun",
        step: "wasm-compare",
        file: WASM_COMPARE_JSON,
        path: &[
            Key("runtimes"),
            Key("bun"),
            Key("results"),
            item("engine", "pglite"),
            Key("cold_init_ms"),
        ],
        envelope: &[Key("runtimes"), Key("bun")],
        unit: Unit::Millis,
        null_means: None,
    },
];

/// What became of a figure's step in this run.
#[derive(Debug)]
enum StepState {
    /// The step was not selected.
    NotRun,
    /// The step ran: its status, whether it exited 0, whether it rewrote the file, and when
    /// it started, which dates an envelope inside the file.
    Ran {
        status: String,
        ok: bool,
        rewrote: bool,
        started: SystemTime,
    },
}

/// Print every quoted figure, read from the files this run wrote.
fn print_figures(root: &Path, outcomes: &[StepRun]) {
    println!("\n=== figures (README, npm README, TRADEOFFS.md) ===");
    for figure in FIGURES {
        let path = root.join(figure.file);
        let state = outcomes
            .iter()
            .find(|o| o.step.name == figure.step)
            .map_or(StepState::NotRun, |o| step_state(o, &path));
        let text = std::fs::read_to_string(&path).ok();
        println!("{}", figure_line(figure, &state, text.as_deref()));
    }
}

/// What a step that ran did, with whether it rewrote `file`: modified at or after the
/// step started, which no earlier run's write can be.
fn step_state(o: &StepRun, file: &Path) -> StepState {
    StepState::Ran {
        status: o.run.status(),
        ok: o.run.ok(),
        rewrote: std::fs::metadata(file)
            .and_then(|m| m.modified())
            .is_ok_and(|modified| modified >= o.started),
        started: o.started,
    }
}

/// What a rewritten file holds for one figure.
#[derive(Debug)]
enum Reading {
    /// The value as its unit prints it, or for a JSON `null` why it is null, and its
    /// envelope's stamp.
    Value {
        value: Result<String, String>,
        stamp: String,
    },
    /// The file was rewritten, but not the figure's envelope — another runtime's slice of
    /// a shared file, when the step stopped before this one's.
    Stale,
    /// Why no value could be read.
    Unreadable(&'static str),
}

/// One figure's line: its value and source, or why it was not refreshed.
///
/// A value is printed only from a file — and an envelope within it — its step rewrote in
/// this run; anything else is what a previous run recorded, and is named as not refreshed
/// rather than shown.
fn figure_line(figure: &Figure, state: &StepState, text: Option<&str>) -> String {
    let source = format!("{} {}", figure.file, path_text(figure.path));
    let not_refreshed = |status: &str, what: String| {
        format!(
            "not refreshed — {} {status} and did not rewrite {what}",
            figure.step
        )
    };
    let reading = match state {
        StepState::NotRun => format!("not refreshed — {} was not run", figure.step),
        StepState::Ran {
            status, rewrote, ..
        } if !rewrote => not_refreshed(status, figure.file.to_owned()),
        StepState::Ran {
            status,
            ok,
            started,
            ..
        } => {
            let caveat = if *ok {
                String::new()
            } else {
                format!(
                    "  ⚠ {} {status} — read its log before quoting this",
                    figure.step
                )
            };
            match read_figure(figure, *started, text) {
                Reading::Value {
                    value: Ok(value),
                    stamp,
                } => format!("{value}   ({source} @ {stamp}){caveat}"),
                Reading::Value {
                    value: Err(why),
                    stamp,
                } => format!("n/a — null in the fresh file: {why}   ({source} @ {stamp}){caveat}"),
                Reading::Stale => not_refreshed(
                    status,
                    format!("{} {}", figure.file, path_text(figure.envelope)),
                ),
                Reading::Unreadable(why) => format!("unreadable — {why}   ({source}){caveat}"),
            }
        }
    };
    format!("{}\n    {reading}", figure.quoted)
}

/// What `text`, a file its step rewrote after `started`, holds for `figure`.
fn read_figure(figure: &Figure, started: SystemTime, text: Option<&str>) -> Reading {
    let Some(text) = text else {
        return Reading::Unreadable("the fresh file could not be read");
    };
    let Ok(doc) = parse_jsonb(text) else {
        return Reading::Unreadable("the fresh file is not valid JSON");
    };
    let Some(envelope) = lookup(&doc, figure.envelope) else {
        return Reading::Unreadable("the fresh file has no such envelope");
    };
    if !figure.envelope.is_empty() && !written_since(envelope, started) {
        return Reading::Stale;
    }
    let Some(value) = lookup(&doc, figure.path) else {
        return Reading::Unreadable("the fresh file has no such key");
    };
    let stamp = match lookup(envelope, &[Key("git_commit")]) {
        Some(JsonbValue::String(s)) => s.clone(),
        _ => "unstamped".to_owned(),
    };
    let value = format_value(value, figure.unit).ok_or_else(|| {
        // the bench's own reason beside the value, else what a null there means
        let beside = figure
            .path
            .split_last()
            .map(|(_, parent)| [parent, &[Key("null_reason")]].concat());
        match beside.as_deref().and_then(|path| lookup(&doc, path)) {
            Some(JsonbValue::String(reason)) => reason.clone(),
            _ => figure
                .null_means
                .unwrap_or("unexpected — its bench writes no null here")
                .to_owned(),
        }
    });
    Reading::Value { value, stamp }
}

/// Whether `envelope`'s `timestamp` (an ISO 8601 UTC instant, as `Date.toISOString()` writes
/// it) is at or after `started`. An envelope with no readable timestamp was not.
fn written_since(envelope: &JsonbValue, started: SystemTime) -> bool {
    let Some(JsonbValue::String(timestamp)) = lookup(envelope, &[Key("timestamp")]) else {
        return false;
    };
    let Some(written) = iso_utc_secs(timestamp) else {
        return false;
    };
    started
        .duration_since(SystemTime::UNIX_EPOCH)
        .is_ok_and(|started| written >= started.as_secs_f64())
}

/// Seconds since the Unix epoch of a `YYYY-MM-DDTHH:MM:SS[.fff]Z` instant, the form
/// `Date.toISOString()` writes; `None` for anything else.
fn iso_utc_secs(text: &str) -> Option<f64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let mut date = date.split('-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let mut time = time.split(':');
    let (hour, minute) = (
        time.next()?.parse::<i64>().ok()?,
        time.next()?.parse::<i64>().ok()?,
    );
    let second = time.next()?.parse::<f64>().ok()?;
    if date.next().is_some()
        || time.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
    {
        return None;
    }
    // Days from the civil date: a year counted from March, so the leap day ends it.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some((days * 86_400 + hour * 3_600 + minute * 60) as f64 + second)
}

/// A key path as the figures list prints it: `a.b[field=value].c`.
fn path_text(path: &[Seg]) -> String {
    let mut text = String::new();
    for seg in path {
        match seg {
            Key(key) => {
                if !text.is_empty() {
                    text.push('.');
                }
                text.push_str(key);
            }
            Item { field, value } => {
                text.push('[');
                text.push_str(field);
                text.push('=');
                text.push_str(value);
                text.push(']');
            }
        }
    }
    text
}

/// The value at `path` within `doc`, if every step along it exists.
fn lookup<'a>(doc: &'a JsonbValue, path: &[Seg]) -> Option<&'a JsonbValue> {
    path.iter().try_fold(doc, |node, seg| match (node, seg) {
        (JsonbValue::Object(entries), Key(key)) => {
            entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        }
        (JsonbValue::Array(items), Item { field, value }) => items.iter().find(
            |item| matches!(lookup(item, &[Key(field)]), Some(JsonbValue::String(s)) if s == value),
        ),
        _ => None,
    })
}

/// A recorded value as `unit` prints it; `None` for a JSON `null`, and a value of the
/// wrong shape printed as recorded rather than coerced.
fn format_value(value: &JsonbValue, unit: Unit) -> Option<String> {
    Some(match (value, unit) {
        (JsonbValue::Null, _) => return None,
        (JsonbValue::Number(n), Unit::Times) => format!("{:.3}×", n.to_f64()),
        (JsonbValue::Number(n), Unit::Millis) => format!("{:.2} ms", n.to_f64()),
        (JsonbValue::Number(n), Unit::Kilobytes) => {
            format!("{:.1} KB ({} bytes)", n.to_f64() / 1024.0, n.to_f64())
        }
        (JsonbValue::Number(n), Unit::Megabytes) => {
            format!(
                "{:.2} MB ({} bytes)",
                n.to_f64() / 1024.0 / 1024.0,
                n.to_f64()
            )
        }
        (JsonbValue::String(s), _) => s.clone(),
        (other, _) => other.to_json_string(),
    })
}

fn on_path(tool: &str) -> bool {
    ok(Command::new("which").arg(tool))
}

/// Run a command for its exit status alone, discarding its output.
fn ok(cmd: &mut Command) -> bool {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

const USAGE: &str = "usage: cargo xtask measure [--only a,b] [--skip a,b] [--reps N]\n\
    \x20                          [--legs a,b] [--scratch-db NAME] [--keep-scratch]\n\
    \x20                          [--dry-run]\n\
    \n\
    \x20 steps: exit-proof, wasm-compare, startup, wasm-init, size-compare, gates";

#[derive(Debug)]
struct Opts {
    /// Print the plan and exit. Every real step writes a results JSON, so this
    /// is the only way to exercise the runner without spending a measurement.
    dry_run: bool,
    only: Option<Vec<String>>,
    skip: Vec<String>,
    reps: u32,
    /// Exit-proof leg subset. Dropping `pglet-server` cuts most of the run's
    /// wall time, which is what makes a higher `--reps` affordable.
    legs: Option<Vec<String>>,
    scratch_db: String,
    keep_scratch: bool,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let known: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
        let mut opts = Self {
            dry_run: false,
            only: None,
            skip: Vec::new(),
            reps: 2,
            legs: None,
            scratch_db: "pglet_exitproof_scratch".to_owned(),
            keep_scratch: false,
        };
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let mut value = |flag: &str| -> Result<String, String> {
                it.next()
                    .cloned()
                    .ok_or_else(|| format!("`{flag}` needs a value"))
            };
            match arg.as_str() {
                "--only" => {
                    opts.only = Some(parse_name_list(
                        &value("--only")?,
                        &known,
                        "--only",
                        "step",
                    )?);
                }
                "--skip" => {
                    opts.skip = parse_name_list(&value("--skip")?, &known, "--skip", "step")?;
                }
                "--reps" => {
                    opts.reps = value("--reps")?
                        .parse()
                        .map_err(|_| "`--reps` needs a number".to_owned())?;
                }
                // Not validated against a leg list here: the bench owns that
                // list and rejects an unknown name itself, and duplicating it
                // is exactly the drift this workspace keeps paying for.
                "--legs" => {
                    opts.legs = Some(
                        value("--legs")?
                            .split(',')
                            .map(|s| s.trim().to_owned())
                            .filter(|s| !s.is_empty())
                            .collect(),
                    );
                }
                "--scratch-db" => opts.scratch_db = value("--scratch-db")?,
                "--keep-scratch" => opts.keep_scratch = true,
                "--dry-run" => opts.dry_run = true,
                other => return Err(format!("unknown flag `{other}`")),
            }
        }
        Ok(opts)
    }

    fn wants(&self, name: &str) -> bool {
        if self.skip.iter().any(|s| s == name) {
            return false;
        }
        self.only
            .as_ref()
            .is_none_or(|only| only.iter().any(|s| s == name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(args: &[&str]) -> Result<Opts, String> {
        Opts::parse(&args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn no_filter_selects_every_step() {
        let opts = opts(&[]).expect("parses");
        assert!(STEPS.iter().all(|s| opts.wants(s.name)));
    }

    #[test]
    fn only_and_skip_compose_with_skip_winning() {
        let opts = opts(&["--only", "startup,gates", "--skip", "gates"]).expect("parses");
        assert!(opts.wants("startup"));
        assert!(!opts.wants("gates"), "skip must beat only");
        assert!(!opts.wants("exit-proof"));
    }

    /// A mistyped step name must change the run's outcome loudly. The whole
    /// point of the filter is to spend the run on the steps asked for,
    /// and a silently-dropped name spends it on a different set.
    #[test]
    fn an_unknown_step_name_is_an_error_on_both_flags() {
        assert!(opts(&["--only", "startupp"]).is_err());
        assert!(opts(&["--skip", "exitproof"]).is_err());
    }

    #[test]
    fn a_flag_missing_its_value_is_an_error_rather_than_a_default() {
        assert!(opts(&["--reps"]).is_err());
        assert!(opts(&["--only"]).is_err());
        assert!(opts(&["--reps", "three"]).is_err());
    }

    /// The exit-proof step is the one carrying environment, and each knob has to
    /// reach the bench under the name the bench reads.
    #[test]
    fn the_exit_proof_step_carries_its_knobs_as_env() {
        let opts =
            opts(&["--reps", "3", "--legs", "pglite,pg", "--scratch-db", "sd"]).expect("parses");
        let cmd = command_for("exit-proof", &opts, Path::new("."));
        let env: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        let get = |key: &str| {
            env.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        assert_eq!(get("PGLET_EXITPROOF_REPS"), "3");
        assert_eq!(get("PGLET_EXITPROOF_LEGS"), "pglite,pg");
        assert!(get("PGLET_EXITPROOF_PG_URL").ends_with("/sd"));
    }

    /// size-compare times nothing, so it runs after every timed step; gates stays last.
    #[test]
    fn size_compare_runs_after_the_timed_steps_and_before_gates() {
        let names: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
        let at = |name: &str| names.iter().position(|n| *n == name).expect("a step");
        for timed in ["exit-proof", "wasm-compare", "startup", "wasm-init"] {
            assert!(at(timed) < at("size-compare"), "{timed} runs first");
        }
        assert_eq!(names.last(), Some(&"gates"));
        let opts = opts(&[]).expect("parses");
        let cmd = command_for("size-compare", &opts, Path::new("."));
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["task", "bench:size-compare:update"]);
    }

    const STAT_BEFORE: &str = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 50 0 25 400 25 0 0 0 0 0\n";

    #[test]
    fn cpu_busy_reads_the_aggregate_line_and_counts_iowait_as_idle() {
        // +30 user +10 system +150 idle +10 iowait: 40 busy of 200
        let after = "cpu  130 0 60 950 60 0 0 0 0 0\ncpu0 1 1 1 1 1 0 0 0 0 0\n";
        let busy = cpu_busy_fraction(STAT_BEFORE, after).expect("reads");
        assert!((busy - 0.2).abs() < 1e-9, "{busy}");
    }

    #[test]
    fn cpu_busy_is_none_without_elapsed_time_or_an_aggregate_line() {
        assert_eq!(cpu_busy_fraction(STAT_BEFORE, STAT_BEFORE), None);
        assert_eq!(cpu_busy_fraction("cpu0 1 2 3 4 5\n", STAT_BEFORE), None);
    }

    #[test]
    fn the_idle_check_warns_on_either_reading_over_its_threshold() {
        assert!(idle_warnings(Some(0.5), Some(0.02), 16).is_empty());
        assert!(idle_warnings(None, None, 16).is_empty());
        // a tenth of 16 CPUs is 1.6; a 4-CPU box is held to one runnable task
        assert!(idle_warnings(Some(1.5), Some(0.02), 16).is_empty());
        assert_eq!(idle_warnings(Some(1.7), Some(0.02), 16).len(), 1);
        assert_eq!(idle_warnings(Some(1.1), Some(0.02), 4).len(), 1);
        let busy = idle_warnings(Some(0.2), Some(0.3), 16);
        assert_eq!(busy.len(), 1);
        assert!(busy[0].contains("30% busy"), "{busy:?}");
    }

    fn figure(step: &str) -> &'static Figure {
        FIGURES.iter().find(|f| f.step == step).expect("a figure")
    }

    fn figure_at(path: &[Seg]) -> &'static Figure {
        FIGURES.iter().find(|f| f.path == path).expect("a figure")
    }

    const EXIT_PROOF_DOC: &str = r#"{"git_commit": "abc1234-dirty", "ratios": {
        "pglite_over_pglet_wasm_tests": {"numerator": "pglite", "value": 8.434834579132222},
        "pg_over_pglet_wasm_tests": {"numerator": "pg", "value": null,
                                     "null_reason": "pg not run"},
        "pg_over_pglet_server_tests": {"numerator": "pg", "value": null}},
        "engines": {"pglite": null}}"#;

    fn ran(ok: bool, rewrote: bool) -> StepState {
        ran_at(ok, rewrote, SystemTime::UNIX_EPOCH)
    }

    fn ran_at(ok: bool, rewrote: bool, started: SystemTime) -> StepState {
        StepState::Ran {
            status: if ok { "ok" } else { "FAIL(1)" }.to_owned(),
            ok,
            rewrote,
            started,
        }
    }

    #[test]
    fn a_fresh_figure_prints_its_value_source_and_stamp() {
        let line = figure_line(figure("exit-proof"), &ran(true, true), Some(EXIT_PROOF_DOC));
        assert!(line.contains("8.435×"), "{line}");
        assert!(
            line.contains(
                "benches/exit_proof_results.json ratios.pglite_over_pglet_wasm_tests.value"
            ),
            "{line}"
        );
        assert!(line.contains("@ abc1234-dirty"), "{line}");
        assert!(!line.contains('⚠'), "{line}");
    }

    #[test]
    fn a_figure_not_rewritten_this_run_prints_no_value() {
        for state in [StepState::NotRun, ran(false, false), ran(true, false)] {
            let line = figure_line(figure("exit-proof"), &state, Some(EXIT_PROOF_DOC));
            assert!(line.contains("not refreshed"), "{line}");
            assert!(
                !line.contains("8.43"),
                "a recorded value is not shown as new: {line}"
            );
        }
    }

    #[test]
    fn a_null_ratio_and_a_failed_step_say_so() {
        let pg = figure_at(&[Key("ratios"), Key("pg_over_pglet_wasm_tests"), Key("value")]);
        let line = figure_line(pg, &ran(true, true), Some(EXIT_PROOF_DOC));
        assert!(
            line.contains("n/a — null in the fresh file: pg not run"),
            "{line}"
        );
        // no `null_reason` beside it: what a null ratio means
        let server = figure_at(&[
            Key("ratios"),
            Key("pg_over_pglet_server_tests"),
            Key("value"),
        ]);
        let line = figure_line(server, &ran(true, true), Some(EXIT_PROOF_DOC));
        assert!(line.contains("a leg failed or was not run"), "{line}");
        // a version is not a ratio, and its null says so
        let pglite = figure_at(&[Key("engines"), Key("pglite")]);
        let line = figure_line(pglite, &ran(true, true), Some(EXIT_PROOF_DOC));
        assert!(
            line.contains("the pglite leg was not selected") && !line.contains("test counts"),
            "{line}"
        );
        let line = figure_line(
            figure("exit-proof"),
            &ran(false, true),
            Some(EXIT_PROOF_DOC),
        );
        assert!(
            line.contains("8.435×") && line.contains("⚠ exit-proof FAIL(1)"),
            "{line}"
        );
    }

    /// A file that cannot be read, one that is not JSON, and one without the key are three
    /// different failures, and the line names which.
    #[test]
    fn an_unreadable_figure_says_why() {
        let fig = figure("exit-proof");
        let line = |text: Option<&str>| figure_line(fig, &ran(true, true), text);
        assert!(line(None).contains("could not be read"), "{}", line(None));
        let bad = line(Some("{\"ratios\": "));
        assert!(bad.contains("not valid JSON"), "{bad}");
        let keyless = line(Some(r#"{"ratios": {}}"#));
        assert!(keyless.contains("no such key"), "{keyless}");
    }

    #[test]
    fn byte_text_and_time_figures_format_by_unit() {
        let doc = r#"{"git_commit": "c", "pglet": {"wasm_gzip_bytes": 662048},
            "pglite": {"version": "0.5.3", "total_gzip_bytes": 5410271}}"#;
        let line = |p: &[Seg]| figure_line(figure_at(p), &ran(true, true), Some(doc));
        assert!(line(&[Key("pglet"), Key("wasm_gzip_bytes")]).contains("646.5 KB (662048 bytes)"));
        assert!(line(&[Key("pglite"), Key("total_gzip_bytes")]).contains("5.16 MB"));
        assert!(line(&[Key("pglite"), Key("version")]).contains("0.5.3   ("));
        let startup = r#"{"git_commit": "c", "results": [
            {"scenario": "btree (in-mem)", "p50": 0.5},
            {"scenario": "memory", "p50": 0.7063399}]}"#;
        let memory = figure_line(
            FIGURES
                .iter()
                .find(|f| f.path.contains(&item("scenario", "memory")))
                .expect("a figure"),
            &ran(true, true),
            Some(startup),
        );
        assert!(
            memory.contains("0.71 ms   (benches/startup_results.json results[scenario=memory].p50"),
            "{memory}"
        );
    }

    /// The mtime gate itself: a file written before the step started is not this run's,
    /// one written after it is, and a missing file is not.
    #[test]
    fn step_state_dates_a_file_against_the_steps_start() {
        let dir =
            std::env::temp_dir().join(format!("pglet_measure_step_state_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("results.json");
        std::fs::write(&file, "{}").expect("write");
        let written = std::fs::metadata(&file)
            .and_then(|m| m.modified())
            .expect("mtime");
        let step = &STEPS[0];
        let at = |started: SystemTime, path: &Path| {
            let o = StepRun {
                step,
                started,
                run: TaskRun {
                    exit: Some(0),
                    signal: None,
                    secs: 0.0,
                    log: PathBuf::new(),
                    output: String::new(),
                },
            };
            matches!(step_state(&o, path), StepState::Ran { rewrote: true, .. })
        };
        assert!(
            at(written - Duration::from_secs(60), &file),
            "written after the start"
        );
        assert!(
            !at(written + Duration::from_secs(60), &file),
            "written before the start"
        );
        assert!(!at(
            written - Duration::from_secs(60),
            &dir.join("absent.json")
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One runtime's slice of the shared wasm-compare file is fresh only if its own
    /// timestamp is: a chain that stopped at node rewrote the file and the deno slice, and
    /// left the node and bun slices a previous run wrote.
    #[test]
    fn a_runtime_slice_older_than_the_step_is_not_refreshed() {
        let doc = r#"{"runtimes": {
            "deno": {"timestamp": "2026-10-04T00:22:03.040Z", "git_commit": "d",
                     "results": [{"engine": "pglet", "cold_init_ms": 12.69}]},
            "node": {"timestamp": "2026-10-01T00:00:00.000Z", "git_commit": "n",
                     "results": [{"engine": "pglet", "cold_init_ms": 11.2}]}}}"#;
        let started = SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_073_000);
        let slice = |runtime: &'static str| {
            let fig = FIGURES
                .iter()
                .find(|f| f.envelope == [Key("runtimes"), Key(runtime)])
                .expect("a figure");
            figure_line(fig, &ran_at(false, true, started), Some(doc))
        };
        let deno = slice("deno");
        assert!(deno.contains("12.69 ms") && deno.contains("@ d)"), "{deno}");
        let node = slice("node");
        assert!(
            node.contains("not refreshed") && !node.contains("11.2"),
            "{node}"
        );
        assert!(
            node.contains("did not rewrite benches/wasm_compare_results.json runtimes.node"),
            "{node}"
        );
        let bun = slice("bun");
        assert!(bun.contains("no such envelope"), "{bun}");
    }

    #[test]
    fn iso_instants_read_as_unix_seconds() {
        let at = iso_utc_secs("2026-10-04T00:22:03.040Z").expect("reads");
        assert!((at - 1_791_073_323.04).abs() < 1e-6, "{at}");
        assert_eq!(iso_utc_secs("1970-01-01T00:00:00Z"), Some(0.0));
        assert_eq!(iso_utc_secs("2000-02-29T23:59:59Z"), Some(951_868_799.0));
        for bad in [
            "2026-10-04T00:22:03",
            "2026-13-04T00:00:00Z",
            "yesterday",
            "2026-10-04Z",
        ] {
            assert_eq!(iso_utc_secs(bad), None, "{bad}");
        }
    }

    /// The list's own shape: every figure names a step this set runs, its envelope is a
    /// prefix of its path, and no two figures read the same key.
    #[test]
    fn every_figure_is_a_steps_and_its_envelope_holds_it() {
        for (i, figure) in FIGURES.iter().enumerate() {
            assert!(
                STEPS.iter().any(|s| s.name == figure.step),
                "{}",
                figure.quoted
            );
            assert!(
                figure.path.starts_with(figure.envelope),
                "{}",
                figure.quoted
            );
            assert!(
                FIGURES[..i]
                    .iter()
                    .all(|f| (f.file, f.path) != (figure.file, figure.path)),
                "{} is listed twice",
                path_text(figure.path)
            );
        }
    }

    /// The key-drift guard: each figure's key path, and its envelope's `timestamp` and
    /// `git_commit`, resolve in the file its bench last committed — so a renamed key or a
    /// mistyped selector fails here rather than printing "unreadable" at the end of a long
    /// run. It covers every figure, `ratios` and `engines` in `exit_proof_results.json`
    /// included.
    #[test]
    fn every_figure_resolves_in_its_committed_file() {
        let committed = |file: &str| match file {
            SIZE_COMPARISON_JSON => include_str!("../../../benches/pglite_size_comparison.json"),
            STARTUP_JSON => include_str!("../../../benches/startup_results.json"),
            WASM_INIT_JSON => include_str!("../../../benches/wasm_init_results.json"),
            WASM_COMPARE_JSON => include_str!("../../../benches/wasm_compare_results.json"),
            EXIT_PROOF_JSON => include_str!("../../../benches/exit_proof_results.json"),
            other => panic!("no committed file listed for {other}"),
        };
        for figure in FIGURES {
            let doc = parse_jsonb(committed(figure.file)).expect("committed JSON parses");
            for path in [
                figure.path,
                &[figure.envelope, &[Key("timestamp")]].concat(),
                &[figure.envelope, &[Key("git_commit")]].concat(),
            ] {
                assert!(
                    lookup(&doc, path).is_some(),
                    "{} {} is not in the committed file",
                    figure.file,
                    path_text(path)
                );
            }
        }
    }

    /// Absent `--legs`, the bench must see no variable at all rather than an
    /// empty one — an empty `PGLET_EXITPROOF_LEGS` would select no legs.
    #[test]
    fn no_legs_flag_sets_no_legs_variable() {
        let opts = opts(&[]).expect("parses");
        let cmd = command_for("exit-proof", &opts, Path::new("."));
        assert!(
            !cmd.get_envs()
                .any(|(k, _)| k.to_string_lossy() == "PGLET_EXITPROOF_LEGS")
        );
    }
}
