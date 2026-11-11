//! `cargo xtask gates` — run the gate set once, under a lock, and report.
//!
//! The gate set is the sequence of `npm run` invocations a change must pass.
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

use crate::runner::{lock_run_dir, npm_run, parse_name_list, repo_root, run_logged};

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
/// prints no counters (a vitest summary, not the `passed=/failed=` shape the parser
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
    let run = run_logged(&mut npm_run(gate.task), root, log, false);
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
/// No counter has a default. Each is read from the lines of the exact shape its harness
/// prints, as many as it prints them (one, one per declared-type leg, or one per test
/// binary), or it is absent, and an absent counter the baseline records fails `--check`
/// ([`compare_counters`]). A line carries a counter's label when it begins with the label
/// followed by whitespace, so a line that merely mentions one (npm's banner echoing the
/// command, a test's own output, an indented line) is never read; a line that carries the
/// label in any other shape makes the counter absent rather than leaving it to the lines
/// that kept their shape, so a harness that changes what it prints reads as a missing
/// counter, never as a zero. Absent rather than an error, too, so a task that dies early
/// still reports its exit code and log path instead of the extractor failing on top of the
/// real failure.
fn extract_metrics(gate: &Gate, text: &str) -> Vec<(Metric, i64)> {
    let mut out = Vec::new();

    if gate.task == "test:rust" {
        // `cargo test` prints one `test result:` line per test binary; sum them.
        let results = shaped_values(text, read_test_result).filter(|r| !r.is_empty());
        if let Some(results) = results {
            out.push((Metric::Passed, results.iter().map(|&(p, _)| p).sum()));
            out.push((Metric::Failed, results.iter().map(|&(_, f)| f).sum()));
        }
        return out;
    }

    if gate.task == "test:keywords" {
        if let Some((probes, keywords)) = shaped_values(text, read_keywords_line).and_then(only_one)
        {
            out.push((Metric::Probes, probes));
            out.push((Metric::Keywords, keywords));
        }
        return out;
    }

    // The harness binaries print one `Passed:` / `Failed:` / `Skipped:` block.
    for metric in [Metric::Passed, Metric::Failed, Metric::Skipped] {
        if !gate.metrics.contains(&metric) {
            continue;
        }
        let label = match metric {
            Metric::Passed => "Passed:",
            Metric::Failed => "Failed:",
            _ => "Skipped:",
        };
        if let Some(value) = shaped_values(text, |l| read_count_line(l, label)).and_then(only_one) {
            out.push((metric, value));
        }
    }

    // The two declared-type legs each print an `Asserted:` line and a `Divergent:` line, in
    // order: text (the executor walker) then extended (the planner walker). They are
    // different walkers feeding different clients and can disagree with each other, so the
    // asserted counts are recorded separately rather than summed; the divergences sum.
    if gate.metrics.contains(&Metric::DeclaredText) {
        if let Some([text_leg, extended_leg]) =
            shaped_values(text, read_asserted_line).and_then(per_leg)
        {
            out.push((Metric::DeclaredText, text_leg));
            out.push((Metric::DeclaredExtended, extended_leg));
        }
        if let Some([text_leg, extended_leg]) =
            shaped_values(text, read_divergent_line).and_then(per_leg)
        {
            out.push((Metric::Divergent, text_leg + extended_leg));
        }
    }

    out
}

/// What one line of a gate's output says about a counter.
#[derive(Debug)]
enum LineRead<T> {
    /// The line doesn't carry the counter's label.
    Unlabeled,
    /// The line carries the label in the counter's exact shape.
    Shaped(T),
    /// The line carries the label in another shape: the harness changed what it prints.
    Reshaped,
}

/// The value of every line carrying a counter's label, in order, or `None` when any of
/// them has another shape.
fn shaped_values<T>(text: &str, read: impl Fn(&str) -> LineRead<T>) -> Option<Vec<T>> {
    let mut values = Vec::new();
    for line in text.lines() {
        match read(line) {
            LineRead::Unlabeled => {}
            LineRead::Shaped(value) => values.push(value),
            LineRead::Reshaped => return None,
        }
    }
    Some(values)
}

/// The one value of a counter its harness prints once.
fn only_one<T>(values: Vec<T>) -> Option<T> {
    let [value] = <[T; 1]>::try_from(values).ok()?;
    Some(value)
}

/// The text leg's and the extended leg's values of a declared-type counter.
fn per_leg(values: Vec<i64>) -> Option<[i64; 2]> {
    values.try_into().ok()
}

/// How `line` reads for the counter labeled `label`: unlabeled unless it begins with the
/// label followed by whitespace, then shaped when `shape` reads the words after it.
fn read_line<'a, T>(
    line: &'a str,
    label: &str,
    shape: impl FnOnce(std::str::SplitWhitespace<'a>) -> Option<T>,
) -> LineRead<T> {
    let Some(rest) = line.strip_prefix(label) else {
        return LineRead::Unlabeled;
    };
    if !rest.starts_with(char::is_whitespace) {
        return LineRead::Unlabeled;
    }
    shape(rest.split_whitespace()).map_or(LineRead::Reshaped, LineRead::Shaped)
}

/// A `cargo test` binary's summary, `test result: ok. 12 passed; 0 failed; …`: its passed
/// and failed counts.
fn read_test_result(line: &str) -> LineRead<(i64, i64)> {
    read_line(line, "test result:", |mut words| {
        let verdict = words.next()?;
        if verdict != "ok." && verdict != "FAILED." {
            return None;
        }
        let passed = words.next()?.parse().ok()?;
        if words.next()? != "passed;" {
            return None;
        }
        let failed = words.next()?.parse().ok()?;
        (words.next()? == "failed;").then_some((passed, failed))
    })
}

/// The keyword sweep's summary, `checked 448 probes across 109 keywords (…)`: its probe and
/// keyword counts.
fn read_keywords_line(line: &str) -> LineRead<(i64, i64)> {
    read_line(line, "checked", |mut words| {
        let probes = words.next()?.parse().ok()?;
        if words.next()? != "probes" || words.next()? != "across" {
            return None;
        }
        let keywords = words.next()?.parse().ok()?;
        (words.next()? == "keywords").then_some((probes, keywords))
    })
}

/// A harness's `Passed:  12` line: the label, then one count, and nothing else.
fn read_count_line(line: &str, label: &str) -> LineRead<i64> {
    read_line(line, label, |mut words| {
        let count = words.next()?.parse().ok()?;
        words.next().is_none().then_some(count)
    })
}

/// The fixture harness's `Asserted:     2144 of 5145 fixtures` line: the asserted count.
fn read_asserted_line(line: &str) -> LineRead<i64> {
    read_line(line, "Asserted:", |mut words| {
        let asserted = words.next()?.parse().ok()?;
        let shaped = words.next()? == "of"
            && words.next()?.parse::<i64>().is_ok()
            && words.next()? == "fixtures"
            && words.next().is_none();
        shaped.then_some(asserted)
    })
}

/// The fixture harness's `Divergent:    0 mismatched, 0 unreadable` line: the mismatched
/// count.
fn read_divergent_line(line: &str) -> LineRead<i64> {
    read_line(line, "Divergent:", |mut words| {
        let mismatched = words.next()?.parse().ok()?;
        let shaped = words.next()? == "mismatched,"
            && words.next()?.parse::<i64>().is_ok()
            && words.next()? == "unreadable"
            && words.next().is_none();
        shaped.then_some(mismatched)
    })
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
    use super::{
        Counters, GATES, Gate, Metric, Mode, compare_counters, extract_metrics, verdict_line,
    };

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

    /// The counters `extract_metrics` reads for the gate named `task`.
    fn extracted(task: &str, text: &str) -> Vec<(Metric, i64)> {
        let gate: &Gate = GATES.iter().find(|g| g.task == task).expect("a gate");
        extract_metrics(gate, text)
    }

    #[test]
    fn the_keyword_counters_are_read_only_from_the_sweeps_own_line() {
        let banner = "\n> test:keywords\n> cargo run -p pglet_tests --bin test_keywords\n\n";
        let summary =
            "checked 448 probes across 109 keywords (5 positions each; 97 probe(s) skipped)\n";
        let mentions = "note: 3 probes across 2 keywords were retried\n  \
                        checked 7 probes across 8 keywords (indented)\n\
                        checked: 9 probes across 10 keywords\n";
        let want = vec![(Metric::Probes, 448), (Metric::Keywords, 109)];
        assert_eq!(
            extracted("test:keywords", &format!("{banner}{mentions}{summary}")),
            want
        );
        assert_eq!(
            extracted("test:keywords", &format!("{banner}{mentions}")),
            vec![]
        );
    }

    #[test]
    fn the_harness_counters_are_read_only_from_lines_of_their_shape() {
        let noise = "\n> test:fixtures\n> echo Asserted: 1 of 9 fixtures\n\
                     \x20 Asserted:     1 of 9 fixtures\nAsserted:9 of 9 fixtures\n\
                     \x20 Divergent:    40 mismatched, 0 unreadable\nDivergent:50 rows\n\
                     a test printed Passed: 98\n  Passed: 97\nPassed:95\n  Failed: 3\n";
        let harness = "Asserted:     5 of 9 fixtures\nDivergent:    1 mismatched, 0 unreadable\n\
                       Asserted:     7 of 9 fixtures\nDivergent:    2 mismatched, 3 unreadable\n\
                       Passed:  9\nFailed:  0\nSkipped: 4\n";
        assert_eq!(
            extracted("test:fixtures", &format!("{noise}{harness}{noise}")),
            vec![
                (Metric::Passed, 9),
                (Metric::Failed, 0),
                (Metric::Skipped, 4),
                (Metric::DeclaredText, 5),
                (Metric::DeclaredExtended, 7),
                (Metric::Divergent, 3),
            ]
        );
    }

    #[test]
    fn the_rust_counters_sum_only_the_test_result_lines() {
        let text = "\n> test:rust\n> cargo test --workspace\n\
                    test result::tests::a_float4_is_the_number_its_text_reads_as ... ok\n\
                    test result: ok. 10 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n\
                    a test printed test result: ok. 500 passed; 500 failed;\n\
                    \x20 test result: ok. 600 passed; 600 failed; 0 ignored\n\
                    test result: FAILED. 3 passed; 2 failed; 0 ignored\n\
                    test result: ok. 4 passed; 0 failed; 0 ignored\n";
        assert_eq!(
            extracted("test:rust", text),
            vec![(Metric::Passed, 17), (Metric::Failed, 2)]
        );
    }

    /// Each counted gate's output as its harness prints it, in the shapes of a passing run's
    /// logs, with lines around the counters that carry no label.
    const SHAPED_OUTPUTS: &[(&str, &str)] = &[
        (
            "test:rust",
            "\n> test:rust\n> cargo test --workspace\n\
             test result::tests::a_float4_is_the_number_its_text_reads_as ... ok\n\
             test result: ok. 137 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; \
             finished in 0.29s\n\
             test result: ok. 0 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; \
             finished in 0.00s\n",
        ),
        (
            "test:fixtures",
            "\n=== Declared column types (text) ===\nAsserted:     2144 of 5145 fixtures\n\
             No assertion: 3 eligible (1 describe no row, 2 error on both engines, 0 never \
             reached), 9 not eligible\nDivergent:    0 mismatched, 0 unreadable\n\
             \n=== Declared column types (extended) ===\nAsserted:     2452 of 5145 fixtures\n\
             Divergent:    0 mismatched, 0 unreadable\n\nPassed:  5145\nFailed:  0\nSkipped: 38\n",
        ),
        ("test:dump", "\n=== Results ===\nPassed: 1\nFailed: 0\n"),
        (
            "test:keywords",
            "=== keyword conformance sweep ===\n\
             checked 448 probes across 109 keywords (5 positions each; 97 probe(s) skipped)\n",
        ),
        ("test:concurrent", "test results\nPassed: 38\nFailed: 0\n"),
    ];

    /// The labels a counter's line begins with, and the counters each one carries.
    const LABELS: &[(&str, &[Metric])] = &[
        ("test result:", &[Metric::Passed, Metric::Failed]),
        ("checked", &[Metric::Probes, Metric::Keywords]),
        ("Passed:", &[Metric::Passed]),
        ("Failed:", &[Metric::Failed]),
        ("Skipped:", &[Metric::Skipped]),
        (
            "Asserted:",
            &[Metric::DeclaredText, Metric::DeclaredExtended],
        ),
        ("Divergent:", &[Metric::Divergent]),
    ];

    /// `--check` of `text` as `task`'s output against `baseline`.
    fn check(task: &str, text: &str, baseline: &[(String, i64)]) -> (Vec<String>, bool) {
        let gate: &Gate = GATES.iter().find(|g| g.task == task).expect("a gate");
        let found = extract_metrics(gate, text);
        let prefix = gate.key();
        let counters = Counters {
            prefix: &prefix,
            declared: gate.metrics,
            found: &found,
        };
        compare_counters(baseline, &[counters])
    }

    /// `task`'s counters read from `text`, recorded as a baseline.
    fn baseline_of(task: &str, text: &str) -> Vec<(String, i64)> {
        let gate: &Gate = GATES.iter().find(|g| g.task == task).expect("a gate");
        let found = extract_metrics(gate, text);
        assert_eq!(
            found.iter().map(|&(m, _)| m).collect::<Vec<_>>(),
            gate.metrics,
            "{task} reads every counter it declares"
        );
        found
            .iter()
            .map(|&(m, v)| (format!("{}.{}", gate.key(), m.key()), v))
            .collect()
    }

    /// That `--check` of `text` as `task`'s output, a passing output changed as `change` says,
    /// fails with each of `metrics` `MISSING`.
    fn assert_missing(
        task: &str,
        change: &str,
        text: &str,
        baseline: &[(String, i64)],
        metrics: &[Metric],
    ) {
        let prefix = GATES.iter().find(|g| g.task == task).expect("a gate").key();
        let (lines, ok) = check(task, text, baseline);
        assert!(!ok, "{task}, {change}: passed --check: {lines:?}");
        for metric in metrics {
            let key = format!("{prefix}.{}", metric.key());
            assert!(
                lines
                    .iter()
                    .any(|l| l.starts_with(&format!("MISSING {key} "))),
                "{task}, {change}: left {key} read: {lines:?}"
            );
        }
    }

    /// Each counter line of each counted gate, changed in a way that keeps its label, removed,
    /// or repeated, and every line of a label removed, makes each counter it carries
    /// `MISSING`, so `--check` fails; the outputs themselves pass against what they read.
    #[test]
    fn a_reshaped_or_missing_counter_line_fails_check() {
        type Reshape = fn(&str, &str) -> String;
        let reshapes: &[(&str, Reshape)] = &[
            ("a word after the label", |label, rest| {
                format!("{label} about{rest}")
            }),
            ("a field at the end", |label, rest| {
                format!("{label}{rest}, 0 late")
            }),
            ("a word at the end", |label, rest| {
                format!("{label}{rest} late")
            }),
            ("a count spelled out", |label, rest| {
                let digit = rest.find(|c: char| c.is_ascii_digit()).expect("a count");
                format!("{label}{}x{}", &rest[..digit], &rest[digit..])
            }),
        ];
        for &(task, text) in SHAPED_OUTPUTS {
            let baseline = baseline_of(task, text);
            let (lines, ok) = check(task, text, &baseline);
            assert!(ok && lines.is_empty(), "{task}: {lines:?}");
            let labeled = text.lines().enumerate().filter_map(|(i, line)| {
                LABELS
                    .iter()
                    .find(|(label, _)| {
                        line.strip_prefix(label)
                            .is_some_and(|r| r.starts_with(char::is_whitespace))
                    })
                    .map(|&(label, metrics)| (i, line, label, metrics))
            });
            let mut probed = 0;
            for (i, line, label, metrics) in labeled {
                probed += 1;
                let rest = &line[label.len()..];
                let mut variants: Vec<(String, Option<String>)> = reshapes
                    .iter()
                    .map(|(name, reshape)| ((*name).to_owned(), Some(reshape(label, rest))))
                    .collect();
                // a counter `cargo test` sums over its binaries can't miss one of several, or
                // tell a repeated one from another binary's
                if label != "test result:" {
                    variants.push(("the line gone".to_owned(), None));
                    variants.push((
                        "the line printed twice".to_owned(),
                        Some(format!("{line}\n{line}")),
                    ));
                }
                for (name, replacement) in variants {
                    // a trailing field after `keywords` and after `failed;` is the shape
                    let trailing_allowed = (name == "a field at the end"
                        || name == "a word at the end")
                        && (label == "checked" || label == "test result:");
                    if trailing_allowed {
                        continue;
                    }
                    let changed: String = text
                        .lines()
                        .enumerate()
                        .filter_map(|(j, l)| {
                            if j == i {
                                replacement.clone()
                            } else {
                                Some(l.to_owned())
                            }
                        })
                        .fold(String::new(), |s, l| s + &l + "\n");
                    assert_missing(
                        task,
                        &format!("{name}: {line:?}"),
                        &changed,
                        &baseline,
                        metrics,
                    );
                }
            }
            assert!(probed > 0, "{task}: no counter line to change");
            for &(label, metrics) in LABELS {
                let kept: String = text
                    .lines()
                    .filter(|l| {
                        !l.strip_prefix(label)
                            .is_some_and(|r| r.starts_with(char::is_whitespace))
                    })
                    .fold(String::new(), |s, l| s + l + "\n");
                if kept.len() == text.len() {
                    continue;
                }
                assert_missing(task, &format!("no {label} line"), &kept, &baseline, metrics);
            }
        }
    }

    /// A third field on one leg's `Divergent:` line once read as no divergence at all, so a
    /// real divergence passed `--check`.
    #[test]
    fn a_divergent_line_with_another_field_is_missing_not_zero() {
        let text = "Asserted:     5 of 9 fixtures\nDivergent:    4 mismatched, 0 unreadable, 0 late\n\
                    Asserted:     7 of 9 fixtures\nDivergent:    0 mismatched, 0 unreadable\n\
                    Passed:  9\nFailed:  0\nSkipped: 0\n";
        let found = extracted("test:fixtures", text);
        assert!(
            found.iter().all(|(m, _)| *m != Metric::Divergent),
            "{found:?}"
        );
    }
}
