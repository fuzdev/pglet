-- A function over `avg` of a column a scalar subquery defines reads the exact `numeric`:
-- `round` rounds `1.5` away from zero and takes a scale, where the `double precision`
-- average rounded to even and `round(…, 1)` had no `numeric` to take.
-- @setup
CREATE TABLE t (k INT, i INT);
INSERT INTO t VALUES (1, 1), (2, 2);
-- @test
SELECT round(avg(x)) AS r, round(avg(x), 1) AS r1, avg(x)::int AS c, avg(x + 1) AS operand,
  avg(y) AS constant
FROM (SELECT (SELECT i FROM t t2 WHERE t2.k = t.k) AS x, (SELECT 1) AS y FROM t) q;
