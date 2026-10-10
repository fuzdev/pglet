-- An unknown literal that is not an integer, beside a computed `integer` branch, is
-- `22P02`, as 18.6 converts it.
-- @test
-- @expect-error: invalid input syntax for type integer
SELECT COALESCE('abc', length('ab'));
