-- A parenless `char` / `character` COLUMN is `char(1)` in Postgres (only bare
-- `varchar` / `character varying` is unlimited). The valid 1-character insert in
-- setup succeeds on both engines; a 2-character value fails loud (Postgres "value
-- too long for type character(1)", SQLSTATE 22001) rather than silently storing it.
-- @setup
CREATE TABLE bc (c char);
INSERT INTO bc VALUES ('a');
-- @test
-- @expect-error: value too long for type character(1)
INSERT INTO bc VALUES ('ab');
