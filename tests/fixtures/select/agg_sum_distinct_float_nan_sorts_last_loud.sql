-- `NaN` sorts above every other float, so `sum(DISTINCT …)` adds `1e308 + 1.7e308`
-- before the `NaN` and overflows from two finite operands. Input order would add the
-- `NaN` first and answer `NaN`, as the plain `sum` does.
-- @test
-- @expect-error: 22003 value out of range: overflow
SELECT sum(DISTINCT x) FROM (VALUES ('NaN'::float8), (1.7e308), (1e308)) v(x);
