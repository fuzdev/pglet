//! Tiny stats (no external crate).
//!
//! **These definitions mirror `@fuzdev/fuz_util`'s `stats.ts`, deliberately and
//! exactly**, because pglet's figures are produced on both sides of a language
//! split and land in one directory of `benches/*_results.json`. A reader
//! comparing a native `p50_us` against a wasm `p50` is comparing two numbers
//! that must mean the same thing; before this they did not. Three definitions
//! were live at once:
//!
//! - this module's `percentile_sorted`, nearest-rank — it *rounded* to a sample
//!   that exists, where `stats_percentile` interpolates (R-7);
//! - `reads.rs`'s own private `median`, which averaged the two middles on an
//!   even count — agreeing with `stats_median` and disagreeing with the
//!   percentile function in the same crate;
//! - `cv`, over a **sample** standard deviation (`n - 1`), where
//!   `stats_std_dev` uses the **population** one (`n`) — so every `cv` in this
//!   crate's JSON read slightly higher than every `cv` in the Deno benches',
//!   for the same spread.
//!
//! None of that is visible in a table: each number is plausible on its own, and
//! only a cross-file comparison exposes it. That is the shape of drift this
//! workspace keeps paying for, so the rules live in one place per language and
//! the tests below pin them to worked examples that a reimplementation on either
//! side has to reproduce.

/// Percentile over an already-sorted slice, by the **R-7** method that
/// `stats_percentile` uses: the rank is `(n - 1) * p` and a fractional rank
/// interpolates linearly between its neighbours.
///
/// `p` in 0.0..=1.0. Empty → `0.0`.
pub fn percentile_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let index = p * (sorted.len() as f64 - 1.0);
    let lower = index.floor();
    let upper = index.ceil();
    let (lo, hi) = (lower as usize, upper as usize);
    let (lo, hi) = (lo.min(sorted.len() - 1), hi.min(sorted.len() - 1));
    if lo == hi {
        return sorted[lo];
    }
    let weight = index - lower;
    sorted[lo].mul_add(1.0 - weight, sorted[hi] * weight)
}

/// Median by `stats_median`'s rule: an even count averages the two middles.
///
/// Takes an unsorted slice and sorts a copy — the callers that already hold a
/// sorted slice can use [`percentile_sorted`] at `0.5`, which agrees with this
/// by construction now that both interpolate.
pub fn median(samples: &[f64]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile_sorted(&sorted, 0.5)
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Population standard deviation (divides by `n`), matching `stats_std_dev`.
fn stddev(xs: &[f64], mean: f64) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / xs.len() as f64;
    var.sqrt()
}

/// Coefficient of variation = stddev / mean.
pub fn cv(xs: &[f64]) -> f64 {
    let m = mean(xs);
    if m == 0.0 {
        return 0.0;
    }
    stddev(xs, m) / m
}

#[cfg(test)]
mod tests {
    use super::{cv, median, percentile_sorted};

    /// The worked example that separates R-7 from nearest-rank: over four
    /// samples the p50 rank is 1.5, which R-7 reads as the midpoint of the two
    /// middles (`stats_median`'s answer) and nearest-rank rounds to one of them.
    #[test]
    fn an_even_count_interpolates_rather_than_rounding() {
        let xs = [1.0, 2.0, 4.0, 8.0];
        assert!((percentile_sorted(&xs, 0.5) - 3.0).abs() < 1e-12);
        assert!((median(&xs) - 3.0).abs() < 1e-12);
    }

    /// An odd count lands on a real sample under either rule, which is why the
    /// even case above is the discriminating one.
    #[test]
    fn an_odd_count_lands_on_the_middle_sample() {
        assert!((percentile_sorted(&[1.0, 2.0, 4.0], 0.5) - 2.0).abs() < 1e-12);
    }

    /// p95 over 21 samples has rank 19.0 exactly — no interpolation — while over
    /// 20 it is 18.05 and reads between two samples.
    #[test]
    fn a_fractional_upper_percentile_interpolates() {
        let xs: Vec<f64> = (0..20).map(f64::from).collect();
        let got = percentile_sorted(&xs, 0.95);
        assert!((got - 18.05).abs() < 1e-12, "got {got}");
    }

    #[test]
    fn the_ends_are_the_ends() {
        let xs = [1.0, 2.0, 4.0, 8.0];
        assert!((percentile_sorted(&xs, 0.0) - 1.0).abs() < 1e-12);
        assert!((percentile_sorted(&xs, 1.0) - 8.0).abs() < 1e-12);
    }

    /// Population, not sample: over `[2, 4, 4, 4, 5, 5, 7, 9]` the population
    /// standard deviation is exactly 2 and the mean exactly 5, so cv is 0.4.
    /// The sample rule would answer ~0.428.
    #[test]
    fn cv_uses_the_population_standard_deviation() {
        let xs = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        assert!((cv(&xs) - 0.4).abs() < 1e-12, "got {}", cv(&xs));
    }

    #[test]
    fn degenerate_inputs_are_zero_rather_than_nan() {
        assert!((percentile_sorted(&[], 0.5) - 0.0).abs() < 1e-12);
        assert!((median(&[]) - 0.0).abs() < 1e-12);
        assert!((cv(&[]) - 0.0).abs() < 1e-12);
        assert!((cv(&[3.0]) - 0.0).abs() < 1e-12);
    }
}
