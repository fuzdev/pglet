-- A `VALUES` list's column, a derived table's aggregate column and a set operation's
-- column are typed branches of a `COALESCE`, `greatest` or `CASE` beside a computed
-- sibling, so the set unifies at the common type and the sibling keeps its value where
-- it was cast to the column's `integer`.
-- @setup
CREATE TABLE t (k INT, f FLOAT8, n NUMERIC);
INSERT INTO t VALUES (1, 1.5, 1.25), (2, 2.5, 2.5), (3, NULL, NULL);
-- @test
SELECT k, COALESCE(v.x, t.f + 0) AS c, greatest(v.x, t.n * 1) AS g,
  CASE WHEN k = 1 THEN s.m ELSE t.f * 2 END AS w, COALESCE(u.y, t.n * 1) AS su
FROM t, (VALUES (NULL::int)) v(x), (SELECT max(k) AS m FROM t WHERE false) s,
  (SELECT k AS y FROM t WHERE false UNION ALL SELECT NULL) u
ORDER BY k;
