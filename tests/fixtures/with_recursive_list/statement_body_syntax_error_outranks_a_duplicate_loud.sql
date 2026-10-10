-- A duplicate name is found once the list has parsed; the statement body's syntax
-- error still outranks it, as 18.6's raw parse has it.
-- @expect-error: the syntax error, not the duplicate (42601)
-- @no-error-fields: 18.6 places this `42601` at the second `FROM` of `SELECT FROM FROM`, pglet at the first.
-- @test
WITH RECURSIVE c AS (SELECT 1),
  c AS (SELECT 2)
SELECT FROM FROM;
