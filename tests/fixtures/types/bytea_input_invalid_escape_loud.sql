-- @test
-- @expect-error: a backslash before a letter is 22P02
SELECT 'a\b'::bytea;
