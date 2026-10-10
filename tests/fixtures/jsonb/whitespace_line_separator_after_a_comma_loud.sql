-- A non-ASCII `White_Space` character after an array comma.
-- @expect-error: a line separator where a token starts
-- @expect-message
-- @test
SELECT (E'[1,\u20282]')::jsonb;
