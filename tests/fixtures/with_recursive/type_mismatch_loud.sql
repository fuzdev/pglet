-- The non-recursive term is authoritative for the column type: an int seed with a
-- numeric-widening recursive term has no single type (integer in the seed, numeric
-- overall) and fails loud.
-- @test
-- @expect-error: recursive column type must match the non-recursive term (42804)
WITH RECURSIVE t(n) AS (
  SELECT 1
  UNION ALL
  SELECT (n + 1)::numeric FROM t WHERE n < 5
)
SELECT n FROM t;
