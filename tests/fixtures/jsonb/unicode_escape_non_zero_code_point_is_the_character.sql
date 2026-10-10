-- The accept side of the code-point-zero refusal: a non-zero code point decodes to its
-- character, in a value and in a key.
-- @test
SELECT '{"k\u0041": "a\u0041b"}'::jsonb #>> '{kA}' AS v;
