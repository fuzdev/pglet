-- A duplicate name outranks a recursive CTE's column-list arity error.
-- @expect-error: the duplicate, not the arity (42712)
-- @expect-message
-- @test
WITH RECURSIVE r(n, m) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  r AS (SELECT 1)
SELECT 1;
