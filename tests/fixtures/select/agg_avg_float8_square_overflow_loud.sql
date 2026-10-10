-- `avg(double precision)` runs `float8_accum`, whose running sum of squares overflows
-- here from finite inputs although their sum is `0`.
-- @test
-- @expect-error: 22003 value out of range: overflow
SELECT avg(x) FROM (VALUES (1e200::float8), (-1e200)) v(x);
