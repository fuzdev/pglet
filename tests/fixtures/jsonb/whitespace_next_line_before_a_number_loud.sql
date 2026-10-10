-- A non-ASCII `White_Space` character before a top-level number.
-- @expect-error: a next-line character where a token starts
-- @expect-message
-- @test
SELECT (E'\u00851')::jsonb;
