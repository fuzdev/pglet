-- @test
-- @expect-error: an octal escape above 377 is 22P02
SELECT '\400'::bytea;
