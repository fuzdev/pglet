-- A later body's syntax error outranks the `ORDER BY` refusal.
-- @expect-error: the syntax error, not the ORDER BY (42601)
-- @no-error-fields: 18.6 places this `42601` at the second `FROM` of `SELECT FROM FROM`, pglet at the first.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 ORDER BY 1),
  q AS (SELECT FROM FROM)
SELECT 1;
