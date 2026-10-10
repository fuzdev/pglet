-- @test
-- @expect-error: an octal escape of two digits is 22P02
SELECT '\12'::bytea;
