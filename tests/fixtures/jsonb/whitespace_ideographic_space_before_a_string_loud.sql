-- A non-ASCII `White_Space` character before a top-level string.
-- @expect-error: an ideographic space where a token starts
-- @expect-message
-- @test
SELECT (E'\u3000"a"')::jsonb;
