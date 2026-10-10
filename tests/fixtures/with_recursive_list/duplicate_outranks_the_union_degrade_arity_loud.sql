-- And a duplicate name outranks that arm's arity error.
-- @expect-error: the duplicate, not the arity (42712)
-- @expect-message
-- @test
WITH RECURSIVE r(n, m) AS (SELECT 1 UNION SELECT 2),
  r AS (SELECT 1)
SELECT 1;
