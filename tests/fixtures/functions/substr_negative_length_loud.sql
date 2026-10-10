-- A negative length is 22011.
-- @test
-- @expect-error: negative substring length not allowed
SELECT substr('abc', 1, -1);
