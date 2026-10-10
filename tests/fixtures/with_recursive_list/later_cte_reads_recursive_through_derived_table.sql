-- A later CTE reading an earlier recursive CTE through a derived table.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT * FROM (SELECT * FROM r) d)
SELECT n FROM q ORDER BY n;
