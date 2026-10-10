-- Three recursive CTEs declared in reverse dependency order, each deduplicating
-- (`UNION`) against its own accumulated rows only.
-- @test
WITH RECURSIVE c3(k) AS (SELECT m FROM c2 UNION SELECT k + 1000 FROM c3 WHERE k < 1000),
  c2(m) AS (SELECT n FROM c1 UNION SELECT m * 10 FROM c2 WHERE m < 100),
  c1(n) AS (SELECT 1 UNION SELECT n + 1 FROM c1 WHERE n < 2)
SELECT k FROM c3 ORDER BY k;
