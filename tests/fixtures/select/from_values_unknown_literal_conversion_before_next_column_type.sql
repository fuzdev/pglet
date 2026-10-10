-- A `VALUES` column's unknown cells are converted before the next column's common type is
-- chosen: column 1's `'x'` is `22P02` ahead of column 2's `integer`/`boolean` `42804`.
-- @expect-error: invalid input syntax for type integer
-- @test
SELECT * FROM (VALUES (1, 1), ('x', true)) v;
