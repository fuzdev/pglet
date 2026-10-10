-- Two independent recursive CTEs in one statement, joined in the body.
-- @test
WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM a WHERE n < 2),
  b(m) AS (SELECT 10 UNION ALL SELECT m + 1 FROM b WHERE m < 11)
SELECT n, m FROM a, b ORDER BY n, m;
