-- A `text` and a `varchar` column on the left, a `name` (`pg_namespace.nspname`, the
-- introspector's operand), in a `WHERE` beside `AND`.
-- @setup
CREATE TYPE mood AS ENUM ('happy', 'sad');
CREATE TABLE rx (t text, v varchar, m mood, i int, b bytea);
INSERT INTO rx VALUES ('pg_x', 'pg_x', 'happy', 1, '\x00');
-- @test
SELECT t ~ '^pg_' AS text_col, v ~ '^pg_' AS varchar_col, t !~ '^pg_' AS negated,
  (SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname !~ '^pg_' AND nspname = 'public') AS name_col
FROM rx WHERE t ~ '^pg' AND t !~ '^zz';
