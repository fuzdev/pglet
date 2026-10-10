/**
 * Self-contained bench stats — median / percentile / mean / CV over a sample
 * array. Deliberately zero-dependency so the cross-runtime benches
 * (`wasm_compare.ts`) run byte-identically under Deno, Node, and Bun without
 * resolving an npm/JSR package (`@fuzdev/fuz_util/stats.ts` exposes a `.ts`
 * subpath that Node won't type-strip inside `node_modules`).
 *
 * **These are `@fuzdev/fuz_util`'s definitions, reproduced because they cannot
 * be imported here — not a second opinion about what a percentile is.** That
 * distinction is the whole reason this comment is long. A local copy for a
 * resolution reason is fine; a local copy that quietly answers differently is a
 * silent divergence between files a reader compares side by side, and pglet had
 * *four* such copies at once (this one, `@fuzdev/fuz_util`'s, and two in the
 * Rust `pglet_bench` — one of which disagreed with its own neighbour). The
 * definitions, all three of which had a dissenter:
 *
 * - percentile is **R-7**: rank `(n - 1) * p`, interpolating between neighbours.
 *   Nearest-rank rounds to a sample that exists,
 *   which reads identically on odd counts and differs on even ones.
 * - median is the p50 of that, so an even count averages the two middles.
 * - `std_dev` is the **population** one (divide by `n`), so `cv` is comparable
 *   with every other `cv` in `benches/*_results.json`.
 *
 * A change here is a change to `crates/pglet_bench/src/stats.rs` and to
 * `@fuzdev/fuz_util`'s `stats.ts`, or it is a bug.
 *
 * All functions copy-and-sort defensively; callers pass raw, unsorted samples.
 */

export const mean = (xs: number[]): number =>
	xs.length === 0 ? 0 : xs.reduce((a, b) => a + b, 0) / xs.length;

/** R-7 percentile (`p` in 0..=1) over an unsorted sample, interpolating. */
export const percentile = (xs: number[], p: number): number => {
	if (xs.length === 0) return 0;
	if (xs.length === 1) return xs[0];
	const sorted = [...xs].sort((a, b) => a - b);
	const index = (sorted.length - 1) * p;
	const lower = Math.floor(index);
	const upper = Math.ceil(index);
	if (lower === upper) return sorted[lower];
	return sorted[lower] * (upper - index) + sorted[upper] * (index - lower);
};

export const median = (xs: number[]): number => percentile(xs, 0.5);

export const min_max = (xs: number[]): {min: number; max: number} => {
	if (xs.length === 0) return {min: 0, max: 0};
	let min = xs[0];
	let max = xs[0];
	for (const x of xs) {
		if (x < min) min = x;
		if (x > max) max = x;
	}
	return {min, max};
};

/** Population standard deviation (divides by `n`), matching `stats_std_dev`. */
export const std_dev = (xs: number[], m: number): number => {
	if (xs.length === 0) return 0;
	const variance = xs.reduce((acc, x) => acc + (x - m) * (x - m), 0) / xs.length;
	return Math.sqrt(variance);
};

/** Coefficient of variation = stddev / mean (0 when mean is 0). */
export const cv = (m: number, sd: number): number => (m === 0 ? 0 : sd / m);

export interface Summary {
	p50: number;
	p95: number;
	mean: number;
	min: number;
	max: number;
	/** Coefficient of variation (stddev / mean). */
	cv: number;
}

/** Summarize a raw sample array into the standard p50/p95/mean/min/max/cv row. */
export const summarize = (samples: number[]): Summary => {
	const m = mean(samples);
	const {min, max} = min_max(samples);
	return {
		p50: median(samples),
		p95: percentile(samples, 0.95),
		mean: m,
		min,
		max,
		cv: cv(m, std_dev(samples, m)),
	};
};
