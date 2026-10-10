-- A JSONB object with a malformed value used to be silently dropped, leaving a
-- bogus-but-"valid" object (`{"a":1}`). It must now fail the whole parse, like
-- Postgres (`invalid input syntax for type json`, SQLSTATE 22P02).
-- @test
-- @expect-error: invalid input syntax for type json
SELECT '{"a":1,"b":@@bad}'::jsonb;
