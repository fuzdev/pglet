-- A no-break space after a quoted element is more of an element, not whitespace.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{"a" }'::text[];
