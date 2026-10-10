-- A scalar subquery aggregating a `VALUES` column is decided on its own run where its
-- argument names no enclosing column: a correlated filter beside it leaves `avg` the
-- exact `numeric`.
-- @setup
CREATE TABLE t (k INT, i INT);
INSERT INTO t VALUES (1, 1), (2, 3), (3, NULL);
-- @test
SELECT k, (SELECT avg(v.x) FROM (VALUES (1), (4), (9007199254740993)) v(x) WHERE v.x > t.i) AS a
FROM t ORDER BY k;
