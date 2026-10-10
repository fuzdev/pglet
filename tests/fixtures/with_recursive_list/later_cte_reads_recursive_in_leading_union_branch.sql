-- A later CTE whose leading `UNION ALL` branch reads an earlier recursive CTE.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT * FROM r UNION ALL SELECT 9)
SELECT n FROM q ORDER BY n;
