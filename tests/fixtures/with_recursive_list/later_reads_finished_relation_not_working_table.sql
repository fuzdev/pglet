-- A later CTE reads the earlier recursive CTE's whole accumulated relation, not the
-- last iteration's working table: `count` sees every row.
-- @test
WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM a WHERE n < 4),
  b(m) AS (SELECT (SELECT count(*) FROM a) UNION ALL SELECT m - 1 FROM b WHERE m > 2)
SELECT m FROM b ORDER BY m;
