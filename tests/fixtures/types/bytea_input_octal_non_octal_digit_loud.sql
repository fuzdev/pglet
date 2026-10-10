-- @test
-- @expect-error: an octal escape holding an 8 is 22P02
SELECT '\128'::bytea;
