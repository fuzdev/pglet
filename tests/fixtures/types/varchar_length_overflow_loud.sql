-- A `varchar(n)` column rejects an over-length value whose excess is not all
-- blanks: `'abcd'` into `varchar(3)` fails loud (Postgres "value too long for type
-- character varying(3)", SQLSTATE 22001) rather than silently truncating.
-- @setup
CREATE TABLE vo (v varchar(3));
-- @test
-- @expect-error: value too long for type character varying(3)
INSERT INTO vo VALUES ('abcd');
