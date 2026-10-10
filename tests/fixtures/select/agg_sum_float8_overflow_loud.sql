-- `sum(double precision)` adds with the `+` operator's overflow check: two finite
-- inputs summing past the type's range are refused, not answered `Infinity`.
-- @test
-- @expect-error: 22003 value out of range: overflow
SELECT sum(x) FROM (VALUES (1e308::float8), (1e308)) v(x);
