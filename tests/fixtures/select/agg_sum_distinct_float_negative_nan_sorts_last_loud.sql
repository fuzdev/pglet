-- A `NaN` with its sign bit set is the same `NaN` to the float order, still above
-- every other value, so the finite inputs are added first and overflow.
-- @test
-- @expect-error: 22003 value out of range: overflow
SELECT sum(DISTINCT x) FROM (VALUES (-'NaN'::float8), (1.7e308), (1e308)) v(x);
