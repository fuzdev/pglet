-- A keyword after `AS` is a column label (`ColLabel` admits every keyword), so it opens no
-- `ORDER BY` / `OFFSET` / `LIMIT` / `FETCH` / locking clause, in the seed or the recursive
-- term.
-- @test
WITH RECURSIVE r(n, a, b, c, d) AS (
  SELECT 1 AS limit, 1 AS order, 1 AS offset, 1 AS for, 1 AS fetch
  UNION ALL
  SELECT n + 1 AS fetch, a AS for, b AS offset, c AS order, d AS limit FROM r WHERE n < 3
)
SELECT n FROM r;
