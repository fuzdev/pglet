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
//! at 40–60% CV under a concurrent wasm build against 0.76–0.88 ms at 6–9% CV
//! with the box idle — so the steps most easily spoiled run while the machine is
//! most likely still quiet, and `gates --check` (which does not care) runs last.
//!
//! Ratios *within* one run survive a noisy box, because every leg pays the same
//! tax; absolute figures and cross-run comparisons do not. That is why the
//! loadavg check below warns rather than refuses — the reading is a 15-minute
//! average and is stale right after a build — and why the summary is worth
//! reading before any figure is copied anywhere.
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

use crate::runner::{TaskRun, deno_task, lock_run_dir, parse_name_list, repo_root, run_logged};

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

    let mut outcomes: Vec<(&Step, TaskRun)> = Vec::with_capacity(selected.len());
    for step in &selected {
        eprintln!("\n== {} — {} ==", step.name, step.title);
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
        outcomes.push((step, outcome));
    }

    teardown(&opts, &outcomes);
    print_summary(&outcomes);
    print_leftover_scratch_dbs();

    let failed: Vec<&str> = outcomes
        .iter()
        .filter(|(_, o)| !o.ok())
        .map(|(s, _)| s.name)
        .collect();
    if failed.is_empty() {
        println!(
            "\nnext: update the README/PLAN figures from the tables above, then commit the\n\
             refreshed benches/*.json"
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
    // (exit-proof and wasm-compare through `build:npm`, `gates` through
    // `size:check`), so this is nearly unconditional.
    if (wants("exit-proof") || wants("wasm-compare") || wants("wasm-init") || wants("gates"))
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
        if !root.join("benches/node_modules").exists() {
            return Err("run `deno task bench:wasm-compare:install` first".to_owned());
        }
    }

    match std::fs::read_to_string("/proc/loadavg") {
        Ok(text) => {
            let fifteen = text
                .split_whitespace()
                .nth(2)
                .and_then(|s| s.parse::<f64>().ok());
            match fifteen {
                Some(load) => {
                    println!("loadavg: {load:.2} (15-min) — this box should be otherwise idle");
                    if load > 1.0 {
                        println!(
                            "  ⚠ above 1.0 — close other work, or expect the absolute figures to flap"
                        );
                    }
                }
                None => println!("loadavg: unreadable"),
            }
        }
        Err(_) => println!("loadavg: unavailable"),
    }
    Ok(())
}

/// Drop the scratch database, unless it is worth keeping.
///
/// It is disposable by construction — the exit-proof bench drops and recreates
/// it before every pass — so the only time its contents are worth anything is
/// when that step failed and left the state the failure happened in.
fn teardown(opts: &Opts, outcomes: &[(&Step, TaskRun)]) {
    let exit_proof_ok = outcomes
        .iter()
        .any(|(s, o)| s.name == "exit-proof" && o.ok());
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

fn print_summary(outcomes: &[(&Step, TaskRun)]) {
    println!("\n=== release measurements ===");
    for (step, outcome) in outcomes {
        println!(
            "{:<14} {:>14} {:>7.0}s  {}",
            step.name,
            outcome.status(),
            outcome.secs,
            outcome.log.display()
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
    \x20 steps: exit-proof, wasm-compare, startup, wasm-init, gates";

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
