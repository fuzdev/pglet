-- One argument names no `col_description` overload: `42883` on both engines.
-- @test
-- @expect-error: col_description(integer) does not exist
SELECT col_description(1);
