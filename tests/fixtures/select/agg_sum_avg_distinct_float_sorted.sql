-- @test
-- `sum`/`avg(DISTINCT …)` over a float add the distinct values in the type's sort
-- order, not in input order, so the rounding is the sorted order's: `-1e16 + 1 + 1e16`
-- is `0`, and `0.1 + 0.2 + 0.3` is `0.6000000000000001`. The plain aggregates keep
-- input order (`1`, `0.6`). `real` sums in `real` and averages in `double precision`.
-- Each aggregate stands alone in its query: beside a `DISTINCT` aggregate, 18.6's
-- planner may sort the input of every aggregate in the query (presorted aggregates).
SELECT
  (SELECT sum(DISTINCT x) FROM (VALUES (1e16::float8), (-1e16), (1)) v(x)) AS sum_d,
  (SELECT avg(DISTINCT x) FROM (VALUES (1e16::float8), (-1e16), (1)) v(x)) AS avg_d,
  (SELECT sum(x) FROM (VALUES (1e16::float8), (-1e16), (1)) v(x)) AS sum_plain,
  (SELECT sum(DISTINCT x) FROM (VALUES (0.3::float8), (0.2), (0.1), (0.3)) v(x)) AS sum_tenths,
  (SELECT avg(DISTINCT x) FROM (VALUES (0.3::float8), (0.2), (0.1), (0.3)) v(x)) AS avg_tenths,
  (SELECT sum(x) FROM (VALUES (0.3::float8), (0.2), (0.1)) v(x)) AS sum_tenths_plain,
  (SELECT sum(DISTINCT x) FROM (VALUES (1e8::float4), (-1e8), (1), (NULL)) v(x)) AS sum_real,
  (SELECT avg(DISTINCT x) FROM (VALUES (1e16::float4), (-1e16), (1), (NULL)) v(x)) AS avg_real,
  (SELECT sum(DISTINCT x) FROM (VALUES (1.7e308::float8), (1e308), (-1e308)) v(x)) AS sum_no_overflow;
