-- A third CTE reading a second that reads the recursive one.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT * FROM r),
  p AS (SELECT n * 2 AS d FROM q)
SELECT d FROM p ORDER BY d;
