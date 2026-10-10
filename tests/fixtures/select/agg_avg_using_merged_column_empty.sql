-- Over no rows the merged column's `avg` is a NULL `numeric`, and the `COALESCE` default is
-- the `numeric` `7` on both legs — where the default was cast to `double precision` while
-- the descriptor beneath said otherwise.
-- @setup
CREATE TABLE t (k INT, i INT);
CREATE TABLE u (k INT, j INT);
INSERT INTO t VALUES (1, 1);
INSERT INTO u VALUES (1, 10);
-- @test
SELECT avg(k), COALESCE(avg(k), 7) FROM t JOIN u USING (k) WHERE false;
