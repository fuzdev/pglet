-- A positional `ORDER BY n` is validated against the OUTPUT column count at plan
-- time, so `3` (the select list has one column) fails loud even though the Sort
-- would sit below the projection (the base row has 3 columns). Postgres: 42P10.
-- @setup
CREATE TABLE t (a INT, b INT, c INT);
INSERT INTO t VALUES (100, 5, 2), (200, 9, 1), (300, 7, 1);

-- @test
-- @expect-error: ORDER BY position 3 is not in select list (42P10)
SELECT a FROM t ORDER BY b, 3;
