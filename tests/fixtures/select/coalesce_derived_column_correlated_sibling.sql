-- A fold in a correlated subquery, over a computed column of the subquery's own derived
-- relation beside an enclosing query's column, is resolved once the enclosing level is
-- known — and the derived column is certain there as it is at its own level, so the fold
-- unifies at the common type, `bigint`, where it cast the computed branch to the
-- enclosing column's `integer` and overflowed.
-- @setup
CREATE TABLE t (k INT, i INT);
INSERT INTO t VALUES (1, 1), (2, NULL), (3, 3);
CREATE TABLE u (k INT, b BIGINT);
INSERT INTO u VALUES (1, NULL), (3, 5000000000);
-- @test
SELECT k,
  (SELECT COALESCE(q.x * 1, t.i) FROM (SELECT b + 0 AS x FROM u WHERE u.k = 3) q) AS derived,
  (SELECT COALESCE(v.x * 1, t.i) FROM (VALUES (5000000000)) v(x)) AS vals,
  (SELECT greatest(s.x + 0, t.i) FROM (SELECT b AS x FROM u WHERE u.k = 3 UNION ALL SELECT NULL) s
    ORDER BY 1 LIMIT 1) AS setop
FROM t ORDER BY k;
