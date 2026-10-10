-- A nonzero numeric below `real`'s smallest subnormal underflows and is refused, not `0`.
-- @test
-- @expect-error: out of range for type real (22003)
-- @expect-message
SELECT 1e-50::numeric::real;
