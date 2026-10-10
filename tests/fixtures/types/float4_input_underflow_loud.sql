-- A nonzero decimal that underflows `real` to zero is refused, not answered `0`.
-- @test
-- @expect-error: "1e-50" is out of range for type real (22003)
-- @expect-message
SELECT '1e-50'::real;
