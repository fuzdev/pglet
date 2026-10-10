-- A JSONB array with a malformed element used to be silently dropped (`[1,3]`).
-- It must now fail the whole parse, like Postgres (`invalid input syntax for
-- type json`, SQLSTATE 22P02).
-- @test
-- @expect-error: invalid input syntax for type json
SELECT '[1,@@bad,3]'::jsonb;
