-- A non-ASCII `White_Space` character after a top-level number.
-- @expect-error: a no-break space where a token starts
-- @expect-message
-- @test
SELECT (E'1\u00a0')::jsonb;
