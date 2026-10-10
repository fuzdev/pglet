-- A column a scalar subquery defines is a typed branch of a `COALESCE`, a `CASE`, a
-- `greatest` or a `nullif` beside a computed sibling, where the subquery's own column
-- is certain: an exact `avg`'s `numeric` and a computed `double precision` fold to
-- `double precision`, and the sibling keeps every digit — cast to the column's `numeric`
-- it kept fifteen. The subquery reads a `VALUES` list and a base table, and stands as
-- the entry, under `+ 1`, a unary minus and `abs`.
-- @setup
CREATE TABLE t (k INT, i INT, f FLOAT8);
INSERT INTO t VALUES (1, 1, 0.1234567890123456789), (2, 4, 1e300);
-- @test
SELECT k, COALESCE(q.x, t.f + 0) AS c, CASE WHEN q.x IS NULL THEN t.f * 1 ELSE q.x END AS w,
  greatest(q.x, t.f + 0) AS g, nullif(t.f + 0, q.x) AS n, COALESCE(q.y, t.f + 0) AS base_table,
  COALESCE(q.p, t.f + 0) AS plus_one, COALESCE(q.m, t.f + 0) AS negated,
  COALESCE(q.a, t.f + 0) AS absolute, COALESCE(q.z, t.f + 0) AS answered
FROM t,
  (SELECT (SELECT avg(x) FROM (VALUES (1), (4)) v(x) WHERE x > 9) AS x,
     (SELECT avg(i) FROM t WHERE i > 9) AS y,
     (SELECT avg(x) FROM (VALUES (1), (4)) v(x) WHERE x > 9) + 1 AS p,
     -(SELECT avg(x) FROM (VALUES (1), (4)) v(x) WHERE x > 9) AS m,
     abs((SELECT avg(x) FROM (VALUES (1), (4)) v(x) WHERE x > 9)) AS a,
     (SELECT avg(x) FROM (VALUES (1), (4)) v(x)) AS z) q
ORDER BY k;
