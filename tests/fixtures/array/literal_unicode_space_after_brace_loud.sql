-- A no-break space after the closing brace is junk.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{1} '::int4[];
