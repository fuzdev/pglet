-- `ONLY` is reserved: alone, it is a syntax error, not a table named `only`.
-- @expect-error: syntax error at end of input
-- @no-error-fields: pglet names no token at the end of the statement
-- @test
SELECT * FROM only;
