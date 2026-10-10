-- The controls beside the multi-owner refusal: once a column is back to one owned sequence —
-- the other dropped, or detached by `OWNED BY NONE` — it answers that one, on both engines.
-- @setup
CREATE TABLE t2 (c int, d serial);
CREATE SEQUENCE zz OWNED BY t2.c;
CREATE SEQUENCE aa OWNED BY t2.c;
CREATE SEQUENCE ab OWNED BY t2.d;
DROP SEQUENCE zz;
ALTER SEQUENCE ab OWNED BY NONE;
-- @test
SELECT pg_get_serial_sequence('t2', 'c') AS after_drop, pg_get_serial_sequence('t2', 'd') AS after_owned_by_none;
