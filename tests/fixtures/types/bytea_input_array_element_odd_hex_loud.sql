-- @test
-- @expect-error: an unquoted element reaching the hex format with an odd digit is 22023
SELECT '{\\x616}'::bytea[];
