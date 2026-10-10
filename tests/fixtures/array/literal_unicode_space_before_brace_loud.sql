-- A no-break space ahead of the opening brace is not whitespace the reader skips.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT ' {a}'::text[];
