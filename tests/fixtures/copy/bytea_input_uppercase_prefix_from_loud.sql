-- A `bytea` field holding `\X61` after COPY's de-escaping is refused by the input function, `22P02`
-- on both engines; the harness compares the two codes.
-- @setup
CREATE TABLE t (id int, b bytea);
-- @copy-from
COPY t FROM STDIN;
-- @data
1	\\X61
\.
-- @verify
SELECT id, b FROM t ORDER BY id;
