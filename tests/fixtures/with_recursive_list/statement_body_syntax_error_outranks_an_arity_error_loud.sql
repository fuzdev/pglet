-- So does a deferred column-list arity error.
-- @expect-error: the syntax error, not the arity (42601)
-- @no-error-fields: 18.6 places this `42601` at the second `FROM` of `SELECT FROM FROM`, pglet at the first.
-- @test
WITH RECURSIVE c(a, b) AS (SELECT 1)
SELECT FROM FROM;
