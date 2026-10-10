-- A body that reads a later-declared CTE is analysed after it, but its syntax error
-- still outranks that CTE's column-list arity error.
-- @expect-error: the syntax error, not the arity (42601)
-- @test
WITH RECURSIVE q AS (SELECT * FROM r WHERE FROM),
  r(n, m) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3)
SELECT 1;
