-- @test
-- @expect-error: a space ahead of `\x` makes it the escape format, where `\x` is an invalid escape, 22P02
SELECT ' \x6162'::bytea;
