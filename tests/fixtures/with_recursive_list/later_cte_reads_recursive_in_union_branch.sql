-- A later CTE whose `UNION ALL` branch after a seed reads an earlier recursive CTE. The
-- read made the later CTE look genuinely recursive, which was refused as a second
-- recursive CTE; it is an ordinary CTE on both engines.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT 10 AS z UNION ALL SELECT n FROM r)
SELECT z FROM q ORDER BY z;
