-- An aggregate over a fold with a `$N` branch reads the fold at its other branches'
-- type: `avg(COALESCE(i, $1))` is the exact `numeric` of an `integer` argument, and
-- `COALESCE(avg(…), $2, min(f) + 0)` 18.6's `double precision`, the `numeric` `avg`
-- converted to it. The parameter left each set at the written reach — `avg` `double
-- precision`, and the three-branch fold `numeric`, its `double precision` branch
-- keeping fifteen digits.
-- @setup
CREATE TABLE t (k INT, i INT, f FLOAT8);
INSERT INTO t VALUES (1, 1, 0.1234567890123456789), (2, NULL, 1e300), (3, 3, 2.5);
-- @param $1 int4 7
-- @param $2 float8 NULL
-- @param $3 int4 9
-- @test
SELECT avg(COALESCE(i, $1)) AS a, COALESCE(avg(CASE WHEN i > 9 THEN i END), $2, min(f) + 0) AS c,
  avg(COALESCE($3, i + 0)) AS first, sum(i) AS s
FROM t;
