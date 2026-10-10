-- A trailing comma is malformed JSON. The old scanner silently accepted it
-- (the empty final segment was skipped); it must now fail loud, like Postgres
-- (`invalid input syntax for type json`, SQLSTATE 22P02).
-- @test
-- @expect-error: invalid input syntax for type json
SELECT '{"a":1,}'::jsonb;
