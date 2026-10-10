-- A three-deep chain declared in reverse: each CTE reads the one declared after it.
-- @test
WITH RECURSIVE c3 AS (SELECT * FROM c2 WHERE m > 1),
  c2(m) AS (SELECT n FROM c1 UNION ALL SELECT m * 10 FROM c2 WHERE m < 100),
  c1(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM c1 WHERE n < 2)
SELECT m FROM c3 ORDER BY m;
