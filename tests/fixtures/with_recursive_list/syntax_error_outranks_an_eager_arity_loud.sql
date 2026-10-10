-- A later body's syntax error outranks a recursive CTE's column-list arity error.
-- @expect-error: the syntax error, not the arity (42601)
-- @no-error-fields: 18.6 places this `42601` at the second `FROM` of `SELECT FROM FROM`, pglet at the first.
-- @test
WITH RECURSIVE r(n, m) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT FROM FROM)
SELECT 1;
