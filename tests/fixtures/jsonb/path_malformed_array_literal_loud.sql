-- The path operand is an array literal read by the array reader: a malformed one is
-- refused, not read as a path with an empty key.
-- @test
-- @expect-error: malformed array literal, 22P02
SELECT '{"a":{"b":1}}'::jsonb #> '{a,,b}';
