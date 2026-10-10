-- Two later CTEs each reading the same recursive CTE, both read by the body.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT * FROM r),
  q2 AS (SELECT n * 2 AS d FROM r)
SELECT n, d FROM q, q2 ORDER BY n, d;
