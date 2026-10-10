-- `avg`/`sum` over a `COALESCE`/`CASE` whose computed branches are of different
-- numeric types: the argument is 18.6's common type of the branches — `integer` with
-- `double precision` is `double precision`, `integer` with `numeric` is `numeric` — so
-- `avg` is `double precision` or `numeric` and every row's value is summed, on rows
-- where the fold answers from either branch.
-- @setup
CREATE TABLE t (i INT, f FLOAT8, n NUMERIC, b BIGINT);
INSERT INTO t VALUES (1, 1.5, 0.5, 10), (NULL, 2.5, 1.25, NULL), (3, NULL, NULL, 30);
-- @test
SELECT avg(COALESCE(i + 0, f + 0)) AS a1, sum(COALESCE(i + 0, f + 0)) AS s1,
  avg(COALESCE(i + 0, n + 0)) AS a2, sum(COALESCE(i + 0, n + 0)) AS s2,
  avg(CASE WHEN i IS NOT NULL THEN i + 0 ELSE f * 2 END) AS a3,
  avg(CASE WHEN i IS NULL THEN length('a') ELSE f + 0 END) AS a4,
  avg(COALESCE(b + 0, f + 0)) AS a5, avg(COALESCE(abs(i), abs(f))) AS a6
FROM t;
