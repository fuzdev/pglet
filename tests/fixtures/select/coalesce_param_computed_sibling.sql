-- A `$N` is of unknown type, and takes its set's target as an untyped constant does: a
-- fold holding one beside a computed branch is read whole, the target 18.6's and the
-- parameter described at it. Each set was left at the written reach, where nothing typed
-- its branches: the parameter was described with no type and its text sent under the
-- other branch's declaration.
-- @setup
CREATE TABLE t (k INT, i INT, f FLOAT8, n NUMERIC);
INSERT INTO t VALUES (1, 1, 0.5, 1.5), (2, NULL, NULL, NULL), (3, 3, 2.5, 2.25);
-- @param $1 float8 7.25
-- @param $2 int4 7
-- @param $3 numeric 7.5
-- @param $4 float8 9.5
-- @param $5 float8 4.5
-- @param $6 numeric 8.5
-- @test
SELECT k, COALESCE($1, f + 0) AS first, COALESCE(i + 0, $2) AS last,
  greatest(n * 1, $3) AS g, CASE WHEN k = 2 THEN $4 ELSE f * 1 END AS w,
  ARRAY[$5, f + 0] AS a, COALESCE(i, $6, n + 0) AS three
FROM t ORDER BY k;
