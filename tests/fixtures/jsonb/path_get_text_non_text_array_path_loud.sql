-- The `#>>` member: the path is a `text[]`, and an `integer[]` matches no signature.
-- @test
-- @expect-error: no jsonb #>> integer[] operator
-- @expect-message
SELECT '{"a":1}'::jsonb #>> ARRAY[1];
