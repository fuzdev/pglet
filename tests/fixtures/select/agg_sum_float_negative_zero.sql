-- @test
-- A float `sum` starts from its first input, so a sum of `-0`s is `-0`; under
-- `DISTINCT` the `-0` and `0` are one value, the one met first. `avg` starts from `0`.
SELECT
  (SELECT sum(x) FROM (VALUES ('-0'::float8), ('-0')) v(x)) AS sum8,
  (SELECT sum(x) FROM (VALUES ('-0'::float4)) v(x)) AS sum4,
  (SELECT sum(DISTINCT x) FROM (VALUES ('-0'::float8), (0)) v(x)) AS distinct_neg_first,
  (SELECT sum(DISTINCT x) FROM (VALUES (0::float8), ('-0')) v(x)) AS distinct_pos_first,
  (SELECT sum(DISTINCT x) FROM (VALUES ('-0'::float4), (0)) v(x)) AS distinct_real,
  (SELECT sum(x) FROM (VALUES ('-0'::float8), (0)) v(x)) AS mixed,
  (SELECT avg(x) FROM (VALUES ('-0'::float8)) v(x)) AS avg8;
