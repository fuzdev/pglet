-- The column number is an `int4`; an `int8` reaches it only through an assignment cast,
-- which function resolution does not apply, so no overload takes it: `42883`.
-- @test
-- @expect-error: col_description(integer, bigint) does not exist
-- @expect-message
SELECT col_description(1, 1::bigint);
