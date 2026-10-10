-- Below half the smallest subnormal a nonzero decimal underflows to zero, and is refused.
-- @test
-- @expect-error: "2e-324" is out of range for type double precision (22003)
-- @expect-message
SELECT '2e-324'::float8;
