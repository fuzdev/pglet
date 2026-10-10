-- The array parser's own body trim, the fourth call site, with its control `[\t]`
-- in `whitespace_json_four_accepted.sql`.
-- @expect-error: a no-break space as an array body
-- @expect-message
-- @test
SELECT (E'[\u00a0]')::jsonb;
