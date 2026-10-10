-- A recursive term reading an earlier recursive CTE's finished relation; it was
-- counted as a second self-reference and refused `42P19`.
-- @test
WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM a WHERE n < 3),
  b(m) AS (SELECT 0 UNION ALL SELECT a.n FROM b JOIN a ON a.n = b.m + 1)
SELECT m FROM b ORDER BY m;
