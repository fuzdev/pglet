-- The narrowing, loud: a metacharacter after the anchor is refused `0A000` here — every pattern but `^`
-- followed by literal characters is (`EDGES.md` row 251). Documentary where 18.6 answers
-- (any pglet error passes); it holds the refusal against a regression that answers wrong.
-- @setup
CREATE TYPE mood AS ENUM ('happy', 'sad');
CREATE TABLE rx (t text, v varchar, m mood, i int, b bytea);
INSERT INTO rx VALUES ('pg_x', 'pg_x', 'happy', 1, '\x00');
-- @test
-- @expect-error: only an anchored literal prefix is supported
SELECT 'a.c' ~ '^a.';
