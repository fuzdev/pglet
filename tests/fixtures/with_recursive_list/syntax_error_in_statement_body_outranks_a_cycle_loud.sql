-- The statement body's syntax error outranks the cycle too.
-- @expect-error: the syntax error, not the cycle (42601)
-- @no-error-fields: 18.6 places this `42601` at the second `FROM` of `SELECT FROM FROM`, pglet at the first.
-- @test
WITH RECURSIVE a AS (SELECT * FROM b),
  b AS (SELECT * FROM a)
SELECT FROM FROM;
