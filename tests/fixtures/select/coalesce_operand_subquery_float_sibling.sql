-- A scalar subquery standing as an operand under a fold's branch is typed where its own
-- column is certain, so the fold is unified: `-(SELECT avg(x) …)` over a `VALUES` column
-- is `numeric`, and beside a computed `double precision` branch each fold is 18.6's
-- `double precision`. With the operand untyped the fold was left unresolved and declared
-- at its first branch's type — the subquery's `numeric` over `double precision` values,
-- and under a `CASE` whose `ELSE` leads, `double precision` over a `numeric` one.
-- @setup
CREATE TABLE t (k INT, f FLOAT8);
INSERT INTO t VALUES (1, 0.1234567890123456789), (2, 1e300), (3, 2.5);
-- @test
SELECT k,
  COALESCE(-(SELECT avg(x) FROM (VALUES (1), (4)) v(x) WHERE x > t.k * 2), t.f + 0) AS c,
  COALESCE(t.f * 1, (SELECT avg(x) FROM (VALUES (1), (4)) v(x)) + 1) AS c2,
  CASE WHEN k = 1 THEN t.f + 0 ELSE -(SELECT avg(x) FROM (VALUES (1), (4)) v(x)) END AS w,
  greatest(abs((SELECT avg(x) FROM (VALUES (1), (4)) v(x))), t.f + 0) AS g,
  least(t.f + 0, (SELECT sum(x) FROM (VALUES (5000000000), (4)) v(x)) * 2) AS l,
  nullif(-(SELECT avg(x) FROM (VALUES (1), (4)) v(x)), t.f + 0) AS n,
  nullif(t.f + 0, (SELECT avg(x) FROM (VALUES (1), (4)) v(x)) * 1) AS n2,
  ARRAY[-(SELECT avg(x) FROM (VALUES (1), (4)) v(x) WHERE x > t.k * 2), t.f + 0] AS a
FROM t ORDER BY k;
