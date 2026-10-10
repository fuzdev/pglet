-- @test
-- `UNION` keeps the first row of each set of equal values, the left side's ahead of
-- the right's, in both orders.
SELECT
  (SELECT x FROM (SELECT 1.0::numeric AS x UNION SELECT 1.00) s) AS numeric_a,
  (SELECT x FROM (SELECT 1.00::numeric AS x UNION SELECT 1.0) s) AS numeric_b,
  (SELECT x FROM (SELECT interval '1 mon' AS x UNION SELECT interval '30 days') s) AS mon_a,
  (SELECT x FROM (SELECT interval '30 days' AS x UNION SELECT interval '1 mon') s) AS mon_b,
  (SELECT x FROM (SELECT '{"a":1}'::jsonb AS x UNION SELECT '{"a":1.0}') s) AS jsonb_a,
  (SELECT x FROM (SELECT '{"a":1.0}'::jsonb AS x UNION SELECT '{"a":1}') s) AS jsonb_b,
  (SELECT x FROM (SELECT ARRAY[1.00] AS x UNION SELECT ARRAY[1.0]) s) AS array_a,
  (SELECT x FROM (SELECT ARRAY[1.0] AS x UNION SELECT ARRAY[1.00]) s) AS array_b,
  (SELECT x FROM (SELECT '-0'::float8 AS x UNION SELECT 0) s) AS zero_a,
  (SELECT x FROM (SELECT 0::float8 AS x UNION SELECT '-0') s) AS zero_b;
