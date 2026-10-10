-- No `left(text, bigint)`: `int8 -> int4` is an assignment cast, so 42883.
-- @test
-- @expect-error: function left(unknown, bigint) does not exist
SELECT left('abc', 2::int8);
