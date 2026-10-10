-- A later CTE aggregating an earlier recursive CTE's rows.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT count(*) AS c, sum(n) AS t FROM r)
SELECT c, t FROM q;
