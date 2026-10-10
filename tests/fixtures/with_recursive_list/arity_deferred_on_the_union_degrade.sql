-- The `UNION`-without-a-self-reference arm is an ordinary CTE, so its arity check is
-- deferred and a later body's syntax error wins.
-- @expect-error: the syntax error, not the arity (42601)
-- @no-error-fields: 18.6 places this `42601` at the second `FROM` of `SELECT FROM FROM`, pglet at the first.
-- @test
WITH RECURSIVE r(n, m) AS (SELECT 1 UNION SELECT 2),
  q AS (SELECT FROM FROM)
SELECT 1;
