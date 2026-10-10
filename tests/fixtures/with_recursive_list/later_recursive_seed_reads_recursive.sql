-- A later CTE that is itself genuinely recursive and reads an earlier recursive CTE
-- in its seed: two recursive CTEs, each iterating to its own fixed point.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q(m) AS (SELECT n FROM r UNION ALL SELECT m + 10 FROM q WHERE m < 5)
SELECT m FROM q ORDER BY m;
