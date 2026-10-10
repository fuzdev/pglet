-- A quoted element the literal ends inside is malformed.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{"a}'::text[];
