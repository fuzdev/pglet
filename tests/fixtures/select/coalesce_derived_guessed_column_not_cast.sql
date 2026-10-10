-- A derived table's or CTE's column whose declared type is the walkers' guess — a
-- `COALESCE` the coercion pass could not unify (a branch over a scalar subquery whose own
-- column is a guess), declared `integer` over a `float8` value
-- — is not a certain operand, so an outer fold reads it at the written reach and does not
-- cast its `3.5` to an `integer`/`bigint` target: each answers 18.6's `3.5`. Cast to
-- `text` so the declared types compared are both `text`.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT, f FLOAT8);
INSERT INTO t VALUES (1, 1, 10, 1.5), (2, NULL, NULL, 2.5);
-- @test
WITH c AS (SELECT k, COALESCE(i + 0, f + (SELECT * FROM unnest(ARRAY[1]))) AS x, b FROM t)
SELECT COALESCE(x * 1, b * 1)::text AS c1, greatest(x + 0, b + 0)::text AS c2,
  COALESCE(x, 0)::text AS c3, nullif(x * 1, b * 1)::text AS c4,
  (CASE WHEN k = 2 THEN x * 1 ELSE b * 1 END)::text AS c5
FROM c ORDER BY k;
