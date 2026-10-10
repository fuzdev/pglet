-- The same refusal on an object key, which decodes through the same string decoder.
-- @test
-- @expect-error: code point zero in a key
-- @expect-message
SELECT '{"k\u0000": 1}'::jsonb;
