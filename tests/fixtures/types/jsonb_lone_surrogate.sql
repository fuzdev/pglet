-- A lone UTF-16 high surrogate (`\uD83D` with no following low surrogate) is an
-- invalid `\uXXXX` escape. The old decoder passed `\uXXXX` through verbatim; the
-- single-pass decoder rejects it, like Postgres (`invalid input syntax for type
-- json`, SQLSTATE 22P02).
-- @test
-- @expect-error: invalid input syntax for type json
SELECT '{"s":"\uD83D"}'::jsonb;
