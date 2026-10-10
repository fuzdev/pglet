-- A set operator's keyword after `AS` is a column label, so it splits the body into no arms:
-- the self-reference stays in the recursive term.
-- @test
WITH RECURSIVE r(n, a, b) AS (
  SELECT 1, 1, 1
  UNION ALL
  SELECT n + 1 AS union, a AS intersect, b AS except FROM r WHERE n < 3
)
SELECT n FROM r;
