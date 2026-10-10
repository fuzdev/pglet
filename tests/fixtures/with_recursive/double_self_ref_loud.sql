-- The recursive term may reference the recursive CTE at most once; a self-join to it
-- (the name appearing twice) is not a valid recursive query. Without this check pglet
-- silently returned rows Postgres rejects.
-- @test
-- @expect-error: self-reference appears more than once (42P19)
WITH RECURSIVE t(n) AS (
  SELECT 1
  UNION ALL
  SELECT (t1.n + t2.n)::int FROM t t1, t t2 WHERE t1.n < 4
)
SELECT n FROM t ORDER BY n;
