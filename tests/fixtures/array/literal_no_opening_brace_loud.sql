-- An array literal starts with its opening brace.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '1'::int4[];
