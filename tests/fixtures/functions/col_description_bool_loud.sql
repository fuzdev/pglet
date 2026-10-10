-- A `boolean` column number names no overload: `42883`.
-- @test
-- @expect-error: col_description(integer, boolean) does not exist
-- @expect-message
SELECT col_description(1, true);
