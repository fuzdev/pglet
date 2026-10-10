-- A later CTE reading an earlier recursive CTE through a scalar subquery.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT (SELECT max(n) FROM r) AS m)
SELECT m FROM q;
