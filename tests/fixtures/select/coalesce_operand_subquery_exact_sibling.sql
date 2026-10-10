-- An operand subquery whose column is certain, beside exact branches: each fold is the
-- common type of every branch, where the untyped operand left the set at the written
-- reach and its branch was cast to the sibling's type — `integer` for `f + (SELECT 1)`,
-- `3.5` answering `4`, and `bigint` for the negated `avg`, `-2.5` answering `-3`.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT, f FLOAT8);
INSERT INTO t VALUES (1, 1, 10, 1.5), (2, NULL, NULL, 2.5), (3, 3, 5000000000, NULL);
-- @test
SELECT k, COALESCE(i, f + (SELECT 1)) AS c, COALESCE(i + 0, b + (SELECT 0)) AS wide,
  COALESCE(-(SELECT avg(x) FROM (VALUES (1), (4)) v(x)), b) AS exact_avg,
  greatest(i, (SELECT max(x) FROM (VALUES (2), (5000000000)) v(x)) - 1) AS g,
  CASE WHEN k = 2 THEN (SELECT 1.5) * 2 ELSE i + 0 END AS w,
  ARRAY[i + 0, f + (SELECT 1)] AS a
FROM t ORDER BY k;
