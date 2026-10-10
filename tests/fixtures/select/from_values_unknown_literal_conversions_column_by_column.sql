-- Each column is unified and its cells converted before the next column's: column 1's
-- `'x'`, in the second row, is refused ahead of column 2's `'a'` in the first row, which
-- a row-by-row conversion would have named.
-- @expect-error: invalid input syntax for type integer
-- @expect-message
-- @test
SELECT * FROM (VALUES (1, 'a'), ('x', 3)) v;
