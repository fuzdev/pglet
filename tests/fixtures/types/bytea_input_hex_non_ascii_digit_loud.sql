-- @test
-- @expect-error: a non-ASCII character among hex digits is 22023
SELECT '\x61é'::bytea;
