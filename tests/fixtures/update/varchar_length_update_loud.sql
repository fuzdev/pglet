-- UPDATE is a second write path through `coerce_column_value`, so a `varchar(n)`
-- column enforces its length on UPDATE too. The in-range UPDATE in setup succeeds
-- (proving the write path accepts a fitting value); setting the column to an
-- over-length non-blank value fails loud (SQLSTATE 22001).
-- @setup
CREATE TABLE uv (id int, v varchar(3));
INSERT INTO uv VALUES (1, 'ab');
UPDATE uv SET v = 'xy' WHERE id = 1;
-- @test
-- @expect-error: value too long for type character varying(3)
UPDATE uv SET v = 'abcd' WHERE id = 1;
