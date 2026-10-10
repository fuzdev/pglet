-- A volatile sibling of a projection `unnest(…)` is evaluated even where the expansion is
-- empty, as 18.6's ProjectSet evaluates every target-list item at its first call: the
-- division fails though no row is emitted.
-- @test
-- @expect-error: division by zero
SELECT unnest(ARRAY[]::int[]), random() + 1/0;
