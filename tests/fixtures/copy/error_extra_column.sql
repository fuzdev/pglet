-- COPY ... FROM STDIN with a data row carrying more fields than the table has
-- columns: both Postgres and pglet must reject it, and with the **same** SQLSTATE —
-- `22P04 extra data after last expected column`. The harness compares the two codes
-- byte for byte where both sides carry one, so naming the class would understate what
-- this fixture asserts.
-- @setup
CREATE TABLE x (a int, b text);
-- @copy-from
COPY x FROM STDIN;
-- @data
1	ok	EXTRA
-- @verify
SELECT a, b FROM x ORDER BY a;
