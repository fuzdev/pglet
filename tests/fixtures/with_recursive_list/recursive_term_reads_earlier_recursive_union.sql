-- The `UNION` spelling: the later recursive CTE deduplicates against its own rows,
-- not against the earlier one's.
-- @test
WITH RECURSIVE a(n) AS (SELECT 1 UNION SELECT n + 1 FROM a WHERE n < 3),
  b(m) AS (SELECT 1 UNION SELECT a.n FROM b JOIN a ON a.n = b.m + 1)
SELECT m FROM b ORDER BY m;
