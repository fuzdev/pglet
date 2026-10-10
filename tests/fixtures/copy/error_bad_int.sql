-- COPY ... FROM STDIN with a non-integer value for an int column: both Postgres and
-- pglet must reject it, and with the **same** SQLSTATE — `22P02 invalid input syntax
-- for type integer`. The harness compares the two codes byte for byte where both sides
-- carry one, so naming the class would understate what this fixture asserts.
-- @setup
CREATE TABLE e (a int, b text);
-- @copy-from
COPY e FROM STDIN;
-- @data
notanint	hello
-- @verify
SELECT a, b FROM e ORDER BY a;
