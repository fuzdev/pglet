-- A recursive term may read an earlier recursive CTE more than once; only its own
-- name is limited to one reference.
-- @test
WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM a WHERE n < 3),
  b(m) AS (SELECT 0 UNION ALL SELECT b.m + 1 FROM b, a, a AS a2 WHERE a.n = b.m + 1 AND a2.n = a.n)
SELECT m FROM b ORDER BY m;
