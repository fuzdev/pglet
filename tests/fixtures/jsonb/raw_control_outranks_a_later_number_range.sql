-- A string's raw control character is raised while the string is lexed; a number's
-- out-of-range is raised after the *following* token has been lexed. So the string first
-- is the string's error.
-- @expect-error: the raw control character in the first element
-- @expect-message
-- @test
SELECT (E'["a\tb", 1e200000]')::jsonb;
