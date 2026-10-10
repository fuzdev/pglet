-- A later CTE filtering an earlier recursive CTE's rows.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT n FROM r WHERE n > 1)
SELECT n FROM q ORDER BY n;
