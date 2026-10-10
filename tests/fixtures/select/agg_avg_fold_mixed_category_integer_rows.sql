-- The same folds over rows where every value comes from the integer branch: the
-- argument's type is still the branches' common type, so `avg` is `double precision`
-- (`2`) or `numeric`, not the exact `numeric` an integer argument would give.
-- @setup
CREATE TABLE t (i INT, f FLOAT8, n NUMERIC, b BIGINT);
INSERT INTO t VALUES (1, 9, 9, 10), (3, 2.5, 2.5, 30);
-- @test
SELECT avg(COALESCE(i + 0, f + 0)) AS a1, sum(COALESCE(i + 0, f + 0)) AS s1,
  avg(COALESCE(i + 0, n + 0)) AS a2, sum(COALESCE(i + 0, n + 0)) AS s2,
  avg(COALESCE(length('ab'), f + 0)) AS a3, avg(COALESCE(b + 0, f + 0)) AS a4
FROM t;
