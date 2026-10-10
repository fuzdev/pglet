-- A single recursive CTE counting up to a fixed point: 1..5.
-- The non-recursive term seeds `1`; the recursive term adds 1 until `n < 5` fails.
-- @test
WITH RECURSIVE t(n) AS (
  SELECT 1
  UNION ALL
  SELECT n + 1 FROM t WHERE n < 5
)
SELECT n FROM t ORDER BY n;
