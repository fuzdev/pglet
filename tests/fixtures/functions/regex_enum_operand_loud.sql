-- No regex operator over an enum: 18.6 has no implicit cast from an enum to `text`. `42883` on both.
-- @setup
CREATE TYPE mood AS ENUM ('happy', 'sad');
CREATE TABLE rx (t text, v varchar, m mood, i int, b bytea);
INSERT INTO rx VALUES ('pg_x', 'pg_x', 'happy', 1, '\x00');
-- @test
-- @expect-error: operator does not exist: mood ~ unknown
SELECT m ~ '^h' FROM rx;
