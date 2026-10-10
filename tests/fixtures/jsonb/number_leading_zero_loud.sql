-- The jsonb number arm enforces the strict JSON number grammar: a leading zero
-- (`007`) is rejected with `22P02` ("invalid input syntax for type json"), matching
-- Postgres's JSON lexer — even though the permissive numeric parser would accept it.
-- @expect-error: leading zero is not a valid JSON number (22P02)
-- @test
SELECT '{"n": 007}'::jsonb;
