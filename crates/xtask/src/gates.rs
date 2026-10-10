//! `cargo xtask gates` — run the gate set once, under a lock, and report.
//!
//! The gate set is the sequence of `deno task` invocations a change must pass.
//! Running it by hand is a shell loop that has to be re-written every time, and
//! three properties kept getting lost in the rewriting. Two of them —
//! exclusivity, and exit codes a killed task cannot fake — are general enough
//! that [`crate::measure`] needs them identically, and they live in
//! [`crate::runner`]; concretely here, two concurrent gate runs fight over the
//! fixture harness's spawned servers and its scratch Postgres schema, and the
//! result is cross-run failures in *both* that read exactly like a real
//! regression — a `@no-txn` fixture's *expected file is stale*, for one. The third is
//! this module's own:
//!
//! **The numbers.** Fixture counts, both declared-type legs and the test total
//! are the figures a change is judged by, and they were being grepped out of
//! logs by hand each time.
//!
//! ```sh
//! cargo xtask gates              # run the set, print a summary
//! cargo xtask gates --check      # ...and fail if a count regressed or went missing
//! cargo xtask gates --update     # ...and record the counts as the baseline
//! cargo xtask gates --only test:fixtures,test:rust
//! ```

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::runner::{deno_task, lock_run_dir, parse_name_list, repo_root, run_logged};

/// The gate set, in run order.
///
/// Cheapest-and-most-likely-to-fail first: `check` catches a compile or lint
/// break in seconds, so a broken tree does not spend the fixture harness's whole
/// run before saying so. That run used to be a quarter of an hour and is now
/// seconds on a warm tree — the harness drives a spawned server over TCP, and
/// the server was leaving Nagle enabled, so nearly all of it was the peer's
/// delayed-ACK timer rather than any work. The ordering rule outlives the
/// figure: a cold tree still pays its compile in `check`, and the next slow gate
/// is only ever one feature away.
///
/// `size:check` sits second for the same reason, and it is here at all because a
/// gate nobody runs enforces nothing: the wasm size cap was once breached by 83 KB and
/// stayed breached, unnoticed, while `size:check` existed and was in no gate set. It prints no counters — the baseline it compares against is
/// `benches/size_baseline.json`, not `tests/gates_baseline.txt` — and it is one of the
/// two gates needing `wasm-pack` on PATH.
///
/// `test:wasm` is the other, and it is here for the same reason `size:check` is: it
/// existed, it was in no gate set, and it had been **red for weeks** — its one transaction
/// test asserted a readable row count after a statement raised inside a block, which
/// only holds on an engine that does not track the aborted block — while everything
/// else ran green. It runs its **own** `wasm:build` — measured at
/// ~15 s warm, the same as `size:check`'s, since wasm-pack re-runs its whole pipeline
/// rather than noticing the artifact is current — against tens of milliseconds for the
/// tests themselves. Reusing the build one gate above would erase that, and is
/// deliberately not done: `--only test:wasm` would then test whatever `pkg/` happened to
/// be on disk, which is the shape of "green from a knob nobody confirmed was wired". It
/// prints no counters (a Deno test summary, not the `passed=/failed=` shape the parser
/// reads), so it gates on its exit code alone.
const GATES: &[Gate] = &[
    Gate::new("check", &[]),
    Gate::new("size:check", &[]),
    Gate::new("test:wasm", &[]),
    Gate::new("test:rust", &[Metric::Passed, Metric::Failed]),
    Gate::new("test:fixtures", FIXTURE_METRICS),
    Gate::new("test:fixtures:profile-lock", FIXTURE_METRICS),
    Gate::new("test:dump", &[Metric::Passed, Metric::Failed]),
    Gate::new("test:pg-dump", &[Metric::Passed, Metric::Failed]),
    Gate::new("test:copy", &[Metric::Passed, Metric::Failed]),
    Gate::new("test:keywords", &[Metric::Probes, Metric::Keywords]),
    Gate::new("test:concurrent", &[Metric::Passed, Metric::Failed]),
];

/// What the fixture harness prints, including both declared-type legs.
const FIXTURE_METRICS: &[Metric] = &[
    Metric::Passed,
    Metric::Failed,
    Metric::Skipped,
    Metric::DeclaredText,
    Metric::DeclaredExtended,
    Metric::Divergent,
];

/// A counter a gate prints and the baseline records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Metric {
    Passed,
    Failed,
    Skipped,
    /// Declared column types asserted on the simple-query leg (the executor walker).
    DeclaredText,
    /// Declared column types asserted on the extended leg (the planner walker).
    DeclaredExtended,
    Divergent,
    Probes,
    Keywords,
}

impl Metric {
    const fn key(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::DeclaredText => "declared_text",
            Self::DeclaredExtended => "declared_extended",
            Self::Divergent => "divergent",
            Self::Probes => "probes",
            Self::Keywords => "keywords",
        }
    }

    /// Whether this counter must be zero rather than merely not-decreasing.
    const fn must_be_zero(self) -> bool {
        matches!(self, Self::Failed | Self::Divergent)
    }
}

#[derive(Debug)]
struct Gate {
    task: &'static str,
    metrics: &'static [Metric],
}

impl Gate {
    const fn new(task: &'static str, metrics: &'static [Metric]) -> Self {
        Self { task, metrics }
    }

    /// The baseline key prefix, e.g. `test:fixtures:profile-lock` -> `fixtures_profile_lock`.
    fn key(&self) -> String {
        self.task
            .strip_prefix("test:")
            .unwrap_or(self.task)
            .replace([':', '-'], "_")
    }
}

/// One gate's outcome.
///
/// Counters keep their [`Metric`] rather than being flattened to strings, so the
/// must-be-zero rule is read from [`Metric::must_be_zero`] in the one place it is
/// defined instead of being re-derived by matching on key suffixes.
#[derive(Debug)]
struct Outcome {
    task: &'static str,
    /// The baseline key prefix for this gate's counters.
    prefix: String,
    /// The counters this gate is *declared* to print, whether or not it did.
    declared: &'static [Metric],
    metrics: Vec<(Metric, i64)>,
    run: crate::runner::TaskRun,
}

impl Outcome {
    const fn ok(&self) -> bool {
        self.run.ok()
    }

    fn status(&self) -> String {
        self.run.status()
    }

    /// This gate's counters as `(baseline key, value)` pairs.
    fn keyed(&self) -> impl Iterator<Item = (String, Metric, i64)> + '_ {
        self.metrics
            .iter()
            .map(|&(metric, value)| (format!("{}.{}", self.prefix, metric.key()), metric, value))
    }

    /// What the baseline comparison reads: the declared counters beside the
    /// reported ones, so a counter the gate never printed is visible as such.
    fn counters(&self) -> Counters<'_> {
        Counters {
            prefix: &self.prefix,
            declared: self.declared,
            found: &self.metrics,
        }
    }
}

/// Entry point for `cargo xtask gates`.
pub fn run(args: &[String]) -> ExitCode {
    let mode = match Mode::parse(args) {
        Ok(mode) => mode,
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("usage: cargo xtask gates [--check | --update] [--only a,b] [--skip a,b]");
            return ExitCode::FAILURE;
        }
    };

    let root = repo_root();
    let run_dir = match lock_run_dir(&root, "gates", "gate run") {
        Ok(dir) => dir,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };

    let selected: Vec<&Gate> = GATES.iter().filter(|g| mode.wants(g.task)).collect();
    if selected.is_empty() {
        eprintln!("error: every gate was filtered out");
        return ExitCode::FAILURE;
    }

    let mut outcomes = Vec::with_capacity(selected.len());
    for gate in selected {
        eprintln!("[gates] {} ...", gate.task);
        let outcome = run_gate(gate, &root, &run_dir.log_dir);
        eprintln!(
            "[gates] {} {} ({:.0}s)",
            gate.task,
            outcome.status(),
            outcome.run.secs
        );
        outcomes.push(outcome);
    }

    print_summary(&outcomes);
    let all_ok = outcomes.iter().all(Outcome::ok);
    // The counter comparison runs *before* the verdict, because it can fail a run
    // whose gates all passed: printed first, "all N gates passed" sat above the
    // MISSING lines that exited the run 1 — a true sentence read as the verdict.
    let counters_ok = match mode.kind {
        ModeKind::Check => compare_baseline(&root, &outcomes),
        ModeKind::Run | ModeKind::Update => true,
    };
    print_verdict(&outcomes, counters_ok);

    if matches!(mode.kind, ModeKind::Update) {
        if !all_ok {
            eprintln!("error: refusing to record a baseline from a run with failures");
            return ExitCode::FAILURE;
        }
        if let Err(e) = write_baseline(&root, &outcomes) {
            eprintln!("error: cannot write the baseline: {e}");
            return ExitCode::FAILURE;
        }
        println!("baseline written to {}", baseline_path(&root).display());
    }

    if all_ok && counters_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Run one gate, writing its combined output to a per-task log.
///
/// Not streamed: the runner already prints a line per gate as it starts and
/// finishes, and the tasks themselves are noisy enough that mirroring them would
/// bury the counters the summary exists to show. The log holds everything.
fn run_gate(gate: &Gate, root: &Path, log_dir: &Path) -> Outcome {
    let log = log_dir.join(format!("{}.log", gate.key()));
    let run = run_logged(&mut deno_task(gate.task), root, log, false);
    Outcome {
        task: gate.task,
        prefix: gate.key(),
        declared: gate.metrics,
        metrics: extract_metrics(gate, &run.output),
        run,
    }
}

/// Pull a gate's declared counters out of its output.
///
/// Deliberately tolerant: a counter the output does not carry is simply absent,
/// so a task that dies early still reports its exit code and log path instead of
/// the extractor failing on top of the real failure.
fn extract_metrics(gate: &Gate, text: &str) -> Vec<(Metric, i64)> {
    let mut out = Vec::new();

    if gate.task == "test:rust" {
        // `cargo test` prints one `test result:` line per test binary; sum them.
        let (mut passed, mut failed) = (0_i64, 0_i64);
        for line in text.lines().filter(|l| l.starts_with("test result:")) {
            passed += count_before(line, "passed");
            failed += count_before(line, "failed");
        }
        out.push((Metric::Passed, passed));
        out.push((Metric::Failed, failed));
        return out;
    }

    if gate.task == "test:keywords" {
        if let Some(line) = text.lines().find(|l| l.contains("probes across")) {
            let numbers: Vec<i64> = line
                .split_whitespace()
                .filter_map(|w| w.parse::<i64>().ok())
                .collect();
            if let Some(&probes) = numbers.first() {
                out.push((Metric::Probes, probes));
            }
            if let Some(&keywords) = numbers.get(1) {
                out.push((Metric::Keywords, keywords));
            }
        }
        return out;
    }

    // The harness binaries share a `Passed:` / `Failed:` / `Skipped:` block.
    for metric in [Metric::Passed, Metric::Failed, Metric::Skipped] {
        if !gate.metrics.contains(&metric) {
            continue;
        }
        let label = match metric {
            Metric::Passed => "Passed:",
            Metric::Failed => "Failed:",
            _ => "Skipped:",
        };
        if let Some(value) = last_number_after(text, label) {
            out.push((metric, value));
        }
    }

    // The two declared-type legs print the same `Asserted:` label, in order:
    // text (the executor walker) then extended (the planner walker). They are
    // different walkers feeding different clients and can disagree with each
    // other, so they are recorded separately rather than summed.
    if gate.metrics.contains(&Metric::DeclaredText) {
        let asserted: Vec<i64> = numbers_after(text, "Asserted:");
        if let Some(&text_leg) = asserted.first() {
            out.push((Metric::DeclaredText, text_leg));
        }
        if let Some(&extended_leg) = asserted.get(1) {
            out.push((Metric::DeclaredExtended, extended_leg));
        }
        let divergent: i64 = numbers_after(text, "Divergent:").iter().sum();
        out.push((Metric::Divergent, divergent));
    }

    out
}

/// The count immediately preceding `word` on a `cargo test` result line.
fn count_before(line: &str, word: &str) -> i64 {
    let mut previous = None;
    for token in line.split_whitespace() {
        if token.trim_end_matches(';') == word {
            return previous.and_then(|p: &str| p.parse().ok()).unwrap_or(0);
        }
        previous = Some(token);
    }
    0
}

/// Every number appearing directly after `label` at the start of a line.
fn numbers_after(text: &str, label: &str) -> Vec<i64> {
    text.lines()
        .filter(|l| l.trim_start().starts_with(label))
        .filter_map(|l| l.split_whitespace().nth(1).and_then(|s| s.parse().ok()))
        .collect()
}

fn last_number_after(text: &str, label: &str) -> Option<i64> {
    numbers_after(text, label).last().copied()
}

/// Print the per-gate table.
///
/// The verdict is [`print_verdict`]'s, deliberately not printed here: under
/// `--check` the counter comparison runs after this and can fail a run every gate
/// passed, so a verdict printed here would be a true sentence in the wrong place.
fn print_summary(outcomes: &[Outcome]) {
    println!("\n=== gates ===");
    for outcome in outcomes {
        let metrics = outcome
            .metrics
            .iter()
            .fold(String::new(), |mut s, (metric, value)| {
                let _ = write!(s, " {}={value}", metric.key());
                s
            });
        println!(
            "{:<28} {:>14} {:>7.0}s{metrics}",
            outcome.task,
            outcome.status(),
            outcome.run.secs
        );
        if !outcome.ok() {
            println!("{:<28} log: {}", "", outcome.run.log.display());
        }
    }
}

/// The run's last line, printed after every check that can decide it — so the
/// bottom of the output and the exit code agree.
///
/// `counters_ok` is `--check`'s verdict, and always `true` for the modes that do
/// not compare: a gate can pass and still leave a baseline counter unreported.
fn print_verdict(outcomes: &[Outcome], counters_ok: bool) {
    let failed: Vec<&str> = outcomes
        .iter()
        .filter(|o| !o.ok())
        .map(|o| o.task)
        .collect();
    println!("{}", verdict_line(&failed, outcomes.len(), counters_ok));
}

/// The verdict's wording, split out from the printing so the three cases are
/// testable: a run whose gates all passed and whose counters did not match is a
/// **failing** run, and saying "all N gates passed" of it is how the misleading
/// line got written in the first place.
fn verdict_line(failed: &[&str], total: usize, counters_ok: bool) -> String {
    if !failed.is_empty() {
        return format!("FAILED: {}", failed.join(", "));
    }
    if counters_ok {
        format!("all {total} gates passed")
    } else {
        format!("all {total} gates ran green, but the counters above did not match the baseline")
    }
}

fn baseline_path(root: &Path) -> PathBuf {
    root.join("tests/gates_baseline.txt")
}

/// Compare this run's counters against the recorded baseline.
fn compare_baseline(root: &Path, outcomes: &[Outcome]) -> bool {
    let path = baseline_path(root);
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!(
            "error: no baseline at {} — run `cargo xtask gates --update` first",
            path.display()
        );
        return false;
    };
    let baseline = parse_baseline(&text);
    let counters: Vec<Counters> = outcomes.iter().map(Outcome::counters).collect();
    let (lines, ok) = compare_counters(&baseline, &counters);
    for line in lines {
        println!("{line}");
    }
    ok
}

/// One gate's counters as the comparison reads them: what it was declared to
/// print, and what it did print.
#[derive(Debug)]
struct Counters<'a> {
    prefix: &'a str,
    declared: &'static [Metric],
    found: &'a [(Metric, i64)],
}

/// Compare a run's counters against a baseline, returning the report lines and
/// whether the run passes.
///
/// Three rules, and the third is the one a skipped gate needs:
///
/// - a counter named `failed` or `divergent` must be zero;
/// - every other counter must **not decrease** — growth is progress (more
///   fixtures, more tests) and is reported so it can be recorded, while a shrink
///   means coverage was lost;
/// - a counter the baseline holds and the run **did not report at all** is a
///   failure, not an absence. Several harnesses exit 0 without their
///   prerequisites — `test:dump` and `test:pg-dump` return early without `psql` /
///   `pg_dump` — and a gate that skipped prints no counter block, so without this
///   rule `--check` compares nothing and the run is green having proved nothing.
///   That is the shape a CI runner missing `postgresql-client` takes, and it is
///   equally the shape of a hand run on a box where a client was never
///   installed.
///
/// The third rule ranges over the gates that actually ran, so `--only` still
/// works: a gate the run did not select contributes no `Counters` and its
/// baseline keys are not asked for.
fn compare_counters(baseline: &[(String, i64)], gates: &[Counters]) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut ok = true;
    for gate in gates {
        for &metric in gate.declared {
            let key = format!("{}.{}", gate.prefix, metric.key());
            let Some(&(_, value)) = gate.found.iter().find(|(m, _)| *m == metric) else {
                if let Some((_, want)) = baseline.iter().find(|(k, _)| *k == key) {
                    lines.push(format!(
                        "MISSING {key} (baseline {want}) — the gate reported no such counter; a \
                         skipped gate proves nothing"
                    ));
                    ok = false;
                }
                continue;
            };
            if metric.must_be_zero() {
                if value != 0 {
                    lines.push(format!("REGRESSION {key} = {value} (must be 0)"));
                    ok = false;
                }
                continue;
            }
            match baseline.iter().find(|(k, _)| *k == key) {
                Some((_, want)) if value < *want => {
                    lines.push(format!("REGRESSION {key} = {value} (baseline {want})"));
                    ok = false;
                }
                Some((_, want)) if value > *want => {
                    lines.push(format!(
                        "ahead {key} = {value} (baseline {want}) — run --update to record"
                    ));
                }
                Some(_) => {}
                None => lines.push(format!("new {key} = {value} — run --update to record")),
            }
        }
    }
    (lines, ok)
}

fn parse_baseline(text: &str) -> Vec<(String, i64)> {
    text.lines()
        .map(|l| l.split_once('#').map_or(l, |(before, _)| before))
        .filter_map(|l| l.split_once('='))
        .filter_map(|(k, v)| v.trim().parse().ok().map(|n: i64| (k.trim().to_owned(), n)))
        .collect()
}

fn write_baseline(root: &Path, outcomes: &[Outcome]) -> std::io::Result<()> {
    let mut s = String::from(
        "# Gate counters, written by `cargo xtask gates --update`.\n\
         #\n\
         # `cargo xtask gates --check` fails when a counter decreases, when any\n\
         # `failed` / `divergent` counter is non-zero, or when a gate that ran did not\n\
         # report a counter recorded here — a gate that skipped for want of a\n\
         # prerequisite proves nothing, and used to pass by printing no counters at\n\
         # all. A counter that grows is progress and is reported rather than failed.\n\
         # Regenerate rather than hand-editing.\n\n",
    );
    for outcome in outcomes {
        for (key, _, value) in outcome.keyed() {
            let _ = writeln!(s, "{key} = {value}");
        }
    }
    std::fs::write(baseline_path(root), s)
}

#[derive(Debug)]
enum ModeKind {
    Run,
    Check,
    Update,
}

#[derive(Debug)]
struct Mode {
    kind: ModeKind,
    only: Option<Vec<String>>,
    skip: Vec<String>,
}

impl Mode {
    fn parse(args: &[String]) -> Result<Self, String> {
        let known: Vec<&str> = GATES.iter().map(|g| g.task).collect();
        let mut kind = ModeKind::Run;
        let mut only = None;
        let mut skip = Vec::new();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let mut value = |flag: &str| -> Result<String, String> {
                it.next()
                    .cloned()
                    .ok_or_else(|| format!("`{flag}` needs a comma-separated task list"))
            };
            match arg.as_str() {
                "--check" => kind = ModeKind::Check,
                "--update" => kind = ModeKind::Update,
                // Validated against the gate list rather than merely filtered:
                // a mistyped `--only test:fixture` used to report "matched no
                // gate", which reads as an empty selection rather than as a
                // typo, and a mistyped name alongside a valid one reported
                // nothing at all — it just silently gated less.
                "--only" => {
                    only = Some(parse_name_list(
                        &value("--only")?,
                        &known,
                        "--only",
                        "gate",
                    )?);
                }
                "--skip" => skip = parse_name_list(&value("--skip")?, &known, "--skip", "gate")?,
                other => return Err(format!("unknown flag `{other}`")),
            }
        }
        Ok(Self { kind, only, skip })
    }

    /// Whether a gate survives the filters. `--skip` wins over `--only`.
    fn wants(&self, task: &str) -> bool {
        if self.skip.iter().any(|s| s == task) {
            return false;
        }
        self.only
            .as_ref()
            .is_none_or(|only| only.iter().any(|t| t == task))
    }
}

#[cfg(test)]
mod tests {
    use super::{Counters, GATES, Metric, Mode, compare_counters, verdict_line};

    fn mode(args: &[&str]) -> Result<Mode, String> {
        Mode::parse(&args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn no_filter_selects_every_gate() {
        let mode = mode(&[]).expect("parses");
        assert!(GATES.iter().all(|g| mode.wants(g.task)));
    }

    #[test]
    fn only_and_skip_compose_with_skip_winning() {
        let mode = mode(&["--only", "check,test:rust", "--skip", "test:rust"]).expect("parses");
        assert!(mode.wants("check"));
        assert!(!mode.wants("test:rust"), "skip must beat only");
        assert!(!mode.wants("test:fixtures"));
    }

    /// The typo this replaced: a filter that merely *matched nothing* reported
    /// an empty selection, and one bad name beside a good one gated less than
    /// asked without saying so.
    #[test]
    fn a_mistyped_gate_name_is_an_error() {
        assert!(mode(&["--only", "test:fixture"]).is_err());
        assert!(mode(&["--only", "check,test:fixture"]).is_err());
        assert!(mode(&["--skip", "sizecheck"]).is_err());
    }

    #[test]
    fn a_flag_missing_its_value_is_an_error() {
        assert!(mode(&["--only"]).is_err());
        assert!(mode(&["--nope"]).is_err());
    }

    /// A run whose gates all pass and whose counters do not match the baseline
    /// exits 1, so its last line must not read as a pass. It used to: the verdict
    /// was printed before the comparison, above the `MISSING` lines that failed
    /// the run.
    #[test]
    fn the_verdict_is_a_pass_only_when_the_counters_matched_too() {
        assert_eq!(verdict_line(&[], 11, true), "all 11 gates passed");
        assert!(!verdict_line(&[], 11, false).contains("gates passed"));
        assert!(verdict_line(&[], 11, false).contains("baseline"));
        // A failed gate outranks both: the tasks are what the reader fixes first.
        assert_eq!(verdict_line(&["test:dump"], 11, true), "FAILED: test:dump");
        assert_eq!(verdict_line(&["test:dump"], 11, false), "FAILED: test:dump");
    }

    fn baseline(pairs: &[(&str, i64)]) -> Vec<(String, i64)> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    /// The hole this closes, measured as a rule rather than as a gate: a gate
    /// that ran but reported none of its counters — what `test:dump` and
    /// `test:pg-dump` do when `psql` / `pg_dump` is absent, exiting 0 — used to
    /// be compared against nothing at all, so `--check` was green over a run that
    /// proved nothing.
    #[test]
    fn a_counter_the_run_did_not_report_fails_check() {
        let base = baseline(&[("dump.passed", 1), ("dump.failed", 0)]);
        let gate = Counters {
            prefix: "dump",
            declared: &[Metric::Passed, Metric::Failed],
            found: &[],
        };
        let (lines, ok) = compare_counters(&base, &[gate]);
        assert!(!ok, "a skipped gate must fail --check");
        assert!(
            lines.iter().any(|l| l.starts_with("MISSING dump.passed")),
            "{lines:?}"
        );
    }

    /// ...and the same run reports green when the counters are there, so the new
    /// rule is not simply failing everything.
    #[test]
    fn a_reported_counter_at_the_baseline_passes() {
        let base = baseline(&[("dump.passed", 1), ("dump.failed", 0)]);
        let found = [(Metric::Passed, 1), (Metric::Failed, 0)];
        let gate = Counters {
            prefix: "dump",
            declared: &[Metric::Passed, Metric::Failed],
            found: &found,
        };
        let (lines, ok) = compare_counters(&base, &[gate]);
        assert!(ok, "{lines:?}");
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// A counter the baseline has never held is still merely `new`: the missing
    /// rule fires on a *recorded* counter that vanished, not on one that has yet
    /// to be recorded, so a gate gaining a counter does not fail the run.
    #[test]
    fn an_unrecorded_counter_is_not_a_failure() {
        let gate = Counters {
            prefix: "dump",
            declared: &[Metric::Passed, Metric::Failed],
            found: &[],
        };
        let (lines, ok) = compare_counters(&[], &[gate]);
        assert!(ok, "{lines:?}");
        assert!(lines.is_empty(), "{lines:?}");
    }

    /// The other three verdicts, unchanged by the missing rule.
    #[test]
    fn a_shrunk_counter_and_a_nonzero_failure_both_fail() {
        let base = baseline(&[("fixtures.passed", 2211), ("fixtures.failed", 0)]);
        let found = [(Metric::Passed, 2210), (Metric::Failed, 3)];
        let gate = Counters {
            prefix: "fixtures",
            declared: &[Metric::Passed, Metric::Failed],
            found: &found,
        };
        let (lines, ok) = compare_counters(&base, &[gate]);
        assert!(!ok);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("REGRESSION fixtures.passed"))
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("REGRESSION fixtures.failed"))
        );

        let grown = [(Metric::Passed, 2212), (Metric::Failed, 0)];
        let gate = Counters {
            prefix: "fixtures",
            declared: &[Metric::Passed, Metric::Failed],
            found: &grown,
        };
        let (lines, ok) = compare_counters(&base, &[gate]);
        assert!(ok, "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("ahead fixtures.passed")));
    }
}
