//! pglet development automation.
//!
//! # Usage
//!
//! ```sh
//! cargo xtask check-release  # Audit: no production binary depends on a dev/test-only crate
//! cargo xtask gates          # Run the gate set once, under a lock, and report
//! cargo xtask measure        # Run the quiet-box measurement set behind the README's figures
//! ```
//!
//! `gates` and `measure` are the two task runners and they share their plumbing
//! ([`runner`]) and nothing else: gates answers *is the engine correct* and runs
//! anywhere, measure answers *what are the numbers* and needs an idle box, a live
//! Postgres and `../fuz_app`. See [`measure`].
//!
//! `gates` is the gate-set runner: it holds an exclusive lock so two runs cannot
//! corrupt each other's results, reports every task's exit status (a killed run's
//! log reads green right up to the kill), and extracts the counters a change is
//! judged by. See [`gates`].
//!
//! `check-release` is the release firewall: it walks each production binary's
//! **runtime** dependency graph (`cargo tree -e normal`, so dev/build edges are
//! excluded) and fails if one reaches the dev-only test toolbox. See
//! [`release`].

use std::process::ExitCode;

mod gates;
mod measure;
mod release;
mod runner;

const USAGE: &str = "usage: cargo xtask <check-release | gates | measure>\n\
    \n\
    \x20 check-release            audit: no production binary reaches a dev/test-only crate\n\
    \x20 gates [--check]          run the gate set under a lock; --check fails on a regressed\n\
    \x20                          or unreported counter\n\
    \x20       [--update]         ...or record this run's counters as the baseline\n\
    \x20       [--only a,b]       ...restricted to the named tasks\n\
    \x20 measure                  run the quiet-box measurement set (needs an idle box,\n\
    \x20                          Postgres on 127.0.0.1:5432, and ../fuz_app)\n\
    \x20       [--only a,b]       ...restricted to the named steps\n\
    \x20       [--skip a,b]       ...or minus the named steps\n\
    \x20       [--reps N]         exit-proof timed passes (default 2)\n\
    \x20       [--legs a,b]       exit-proof leg subset (default all four)\n\
    \x20       [--dry-run]        ...print the plan and run nothing (every bench step\n\
    \x20                          overwrites a recorded measurement)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check-release") => release::run(),
        Some("gates") => gates::run(&args[1..]),
        Some("measure") => measure::run(&args[1..]),
        None | Some("help" | "-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown xtask subcommand `{other}`");
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}
