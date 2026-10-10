-- Under `RECURSIVE` every name in the list is visible to every body, so a CTE may read
-- one declared after it; the list is resolved in dependency order.
-- @test
WITH RECURSIVE q AS (SELECT * FROM r),
  r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3)
SELECT n FROM q ORDER BY n;
