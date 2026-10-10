-- `real`'s `sum(DISTINCT …)` sorts as `double precision` does: `2e38 + 3e38`
-- overflows `real` before the `NaN` is reached.
-- @test
-- @expect-error: 22003 value out of range: overflow
SELECT sum(DISTINCT x) FROM (VALUES ('NaN'::float4), (3e38::float4), (2e38::float4)) v(x);
