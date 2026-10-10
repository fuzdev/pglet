-- The unknown cell is converted during parse analysis, so a constant the column type
-- refuses is `22P02` ahead of the `22012` the first row raises only when it is evaluated.
-- @expect-error: invalid input syntax for type integer
-- @expect-message
-- @test
SELECT * FROM (VALUES (1 / 0), ('x')) v(a);
