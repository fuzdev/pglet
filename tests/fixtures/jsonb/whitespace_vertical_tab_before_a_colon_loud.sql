-- The same character class at another trimmed position -- between a key and its colon.
-- @expect-error: a vertical tab where a token starts
-- @expect-message
-- @test
SELECT (E'{"k"\x0b:1}')::jsonb;
