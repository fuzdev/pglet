/**
 * The ratios the exit-proof run records — each one a figure the README quotes.
 *
 * `exit_proof.ts` computes these from its legs' p50s, prints them, and writes them under
 * `ratios` in `exit_proof_results.json`, so a quoted multiple is read out of the run that
 * measured it rather than divided by hand afterwards; `cargo xtask measure` prints them
 * again, by key, in its closing figures list. A separate module from the bench so the
 * arithmetic can be exercised over a recorded document without running a suite.
 *
 * @module
 */

/** One leg's timings, the part of a recorded `results` entry a ratio reads. */
export interface ExitProofLegTimes {
	name: string;
	leg_failed: boolean;
	/** The tests the leg passed; two legs are compared only over the same count. */
	passed: number;
	total: {p50: number};
	tests: {p50: number};
}

/** What one recorded ratio is: `numerator`'s p50 over `denominator`'s, in one bucket. */
export interface ExitProofRatioSpec {
	numerator: string;
	denominator: string;
	/** `tests` (the engine bucket) or `total` (vitest's whole wall time). */
	bucket: 'tests' | 'total';
	/**
	 * Whether the pair crosses the L1 (in-process) / L2 (out-of-process) line the bench
	 * otherwise never compares across — quoted only labelled as such.
	 */
	crosses_tiers: boolean;
}

/** A recorded ratio: its spec, its `value`, and why the value is `null` when it is. */
export interface ExitProofRatio extends ExitProofRatioSpec {
	/** `null` when either leg failed or was not run, or the two passed different test counts. */
	value: number | null;
	/** Why `value` is `null`, naming the legs that left it so; `null` beside a value. */
	null_reason: string | null;
}

/**
 * The quoted ratios, by the key each is recorded under. The keys are a contract with
 * `crates/xtask/src/measure.rs`'s figures list, which reads them back by name.
 */
export const EXIT_PROOF_RATIOS: Record<string, ExitProofRatioSpec> = {
	pglite_over_pglet_wasm_tests: {
		numerator: 'pglite',
		denominator: 'pglet-wasm',
		bucket: 'tests',
		crosses_tiers: false,
	},
	pg_over_pglet_wasm_tests: {
		numerator: 'pg',
		denominator: 'pglet-wasm',
		bucket: 'tests',
		crosses_tiers: true,
	},
	pg_over_pglet_server_tests: {
		numerator: 'pg',
		denominator: 'pglet-server',
		bucket: 'tests',
		crosses_tiers: false,
	},
	pglite_over_pglet_wasm_total: {
		numerator: 'pglite',
		denominator: 'pglet-wasm',
		bucket: 'total',
		crosses_tiers: false,
	},
};

/** A leg's p50 in `bucket`, or `null` when the leg was not run, failed, or has no finite one. */
const p50_of = (
	legs: ExitProofLegTimes[],
	name: string,
	bucket: ExitProofRatioSpec['bucket'],
): number | null => {
	const leg = legs.find((l) => l.name === name);
	if (!leg || leg.leg_failed) return null;
	const p50 = leg[bucket].p50;
	return Number.isFinite(p50) && p50 > 0 ? p50 : null;
};

/**
 * Why a ratio over `legs` has no value, or `null` when it has one: a leg that was not run,
 * failed, or has no p50 in the ratio's bucket — or two green legs that passed different
 * test counts, which ran different work and do not divide into a speed ratio.
 */
const ratio_missing = (legs: ExitProofLegTimes[], ratio: ExitProofRatioSpec): string | null => {
	const reasons: string[] = [];
	for (const name of [ratio.numerator, ratio.denominator]) {
		const leg = legs.find((l) => l.name === name);
		if (!leg) reasons.push(`${name} not run`);
		else if (leg.leg_failed) reasons.push(`${name} failed`);
		else if (p50_of(legs, name, ratio.bucket) === null) reasons.push(`${name} has no ${ratio.bucket} p50`);
	}
	if (reasons.length) return reasons.join(', ');
	const numerator = legs.find((l) => l.name === ratio.numerator);
	const denominator = legs.find((l) => l.name === ratio.denominator);
	if (numerator && denominator && numerator.passed !== denominator.passed) {
		return (
			`legs passed different test counts (${numerator.name} ${numerator.passed}, ` +
			`${denominator.name} ${denominator.passed}) — not the same test set`
		);
	}
	return null;
};

/**
 * Every quoted ratio over `legs`. A ratio over a leg that failed or was not selected, or
 * over two legs that passed different test counts, is `null` with its reason — never `NaN`,
 * a quotient of a failed leg's partial numbers, or one of two different test sets.
 *
 * Recorded unrounded, as the p50s it divides are: a value rounded here and again where it is
 * quoted can round the other way (7.846 recorded as 7.85 reads as ~7.9).
 */
export const exit_proof_ratios = (legs: ExitProofLegTimes[]): Record<string, ExitProofRatio> => {
	const ratios: Record<string, ExitProofRatio> = {};
	for (const [key, spec] of Object.entries(EXIT_PROOF_RATIOS)) {
		const null_reason = ratio_missing(legs, spec);
		const numerator = p50_of(legs, spec.numerator, spec.bucket);
		const denominator = p50_of(legs, spec.denominator, spec.bucket);
		ratios[key] = {
			...spec,
			value:
				null_reason !== null || numerator === null || denominator === null
					? null
					: numerator / denominator,
			null_reason,
		};
	}
	return ratios;
};
