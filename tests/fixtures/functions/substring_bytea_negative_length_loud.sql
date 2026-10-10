-- The FROM … FOR spelling over bytea with a negative length is 22011.
-- @test
-- @expect-error: negative substring length not allowed
SELECT substring('\x6162'::bytea FROM 1 FOR -1);
