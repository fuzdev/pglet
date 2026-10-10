-- @test
-- @expect-error: whitespace between the two digits of a pair is 22023
SELECT '\x6 162'::bytea;
