-- A derived table's column that is a `COALESCE` of an `integer` and a `double precision`
-- computed branch is `double precision`, so `avg` over it is too.
-- @setup
CREATE TABLE t (i INT, f FLOAT8);
INSERT INTO t VALUES (1, 1.5), (NULL, 2.5), (3, NULL);
-- @test
SELECT avg(x) AS a, sum(x) AS s FROM (SELECT COALESCE(i + 0, f + 0) AS x FROM t) d;
