-- A `JOIN … USING` column merged from two same-typed `integer` columns is certainly
-- `integer`, so `avg` over it is 18.6's exact `numeric` and `sum` its `bigint` — on both
-- legs, and with the `COALESCE` default beside it unified to `numeric`.
-- @setup
CREATE TABLE t (k INT, i INT);
CREATE TABLE u (k INT, j INT);
INSERT INTO t VALUES (1, 1), (2, NULL), (3, 5);
INSERT INTO u VALUES (1, 10), (3, 30), (5, 50);
-- @test
SELECT avg(k), sum(k), COALESCE(avg(k), 7) FROM t JOIN u USING (k);
