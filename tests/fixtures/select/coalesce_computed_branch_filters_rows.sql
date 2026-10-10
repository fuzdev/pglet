-- A `WHERE` over a `COALESCE` with a computed `double precision` branch compares that
-- branch's own value: the `2.5` row matches, where a cast to the `integer` sibling's
-- type made it `2`.
-- @setup
CREATE TABLE t (i INT, f FLOAT8);
INSERT INTO t VALUES (1, 1.5), (NULL, 2.5), (3, NULL);
-- @test
SELECT f FROM t WHERE COALESCE(i, f + 0) = 2.5;
