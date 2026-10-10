-- A column's unknown constants are converted before the next column's common type is
-- chosen: column 1's `'x'` is `22P02` ahead of column 2's `integer`/`boolean` `42804`.
-- @expect-error: invalid input syntax for type integer
-- @test
SELECT 1, 1 UNION ALL SELECT 'x', true;
