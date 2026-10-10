-- Two independent recursive CTEs, one deduplicating (`UNION`) over a cycle and one
-- keeping every row (`UNION ALL`): each keeps its own accumulated rows.
-- @test
WITH RECURSIVE a(n) AS (SELECT 1 UNION SELECT (n % 3) + 1 FROM a),
  b(m) AS (SELECT 5 UNION ALL SELECT m + 1 FROM b WHERE m < 6)
SELECT n, m FROM a, b ORDER BY n, m;
