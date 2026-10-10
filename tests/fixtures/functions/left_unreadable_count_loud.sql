-- A string-literal count is resolved to int4, so a spelling int4 cannot read is 22P02.
-- @test
-- @expect-error: invalid input syntax for type integer
SELECT left('abc', 'x');
