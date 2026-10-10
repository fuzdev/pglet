-- A negative length over bytea is 22011 too.
-- @test
-- @expect-error: negative substring length not allowed
SELECT substr('\x6162'::bytea, 1, -1);
