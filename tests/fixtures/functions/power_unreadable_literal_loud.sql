-- A string literal resolved to float8 that float8 cannot read: 22P02.
-- @test
-- @expect-error: invalid input syntax for type double precision
SELECT power('a', 2);
