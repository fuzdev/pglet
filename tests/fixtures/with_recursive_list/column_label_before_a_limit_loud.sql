-- A keyword label does not hide a real clause after it: the `LIMIT` over the recursive query
-- is still 18.6's refusal.
-- @expect-error:LIMIT in a recursive query
-- @test
WITH RECURSIVE r(n) AS (
  SELECT 1
  UNION ALL
  SELECT n + 1 AS limit FROM r WHERE n < 3 LIMIT 1
)
SELECT n FROM r;
