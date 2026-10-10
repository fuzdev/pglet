-- Two recursive CTEs of which the body reads one.
-- @test
WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM a WHERE n < 3),
  b(m) AS (SELECT 1 UNION ALL SELECT m + 1 FROM b WHERE m < 2)
SELECT m FROM b ORDER BY m;
