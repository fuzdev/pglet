-- A wildcard subquery over a FROM `unnest` of a cast array is a certain column, and as
-- the branch of a fold it is typed by that: the fold is the common type of both
-- branches, and a column it defines certain — `avg` over it the exact `numeric`. The
-- branch reader that types a subquery from its column references declines a wildcard
-- over a function scan, and the fold was left unresolved.
-- @setup
CREATE TABLE t (k INT, i INT);
INSERT INTO t VALUES (1, 1), (2, NULL), (3, 3);
-- @test
SELECT COALESCE((SELECT * FROM unnest('{1.5}'::float8[])), 2.5) AS c,
  greatest((SELECT * FROM unnest('{1}'::int[])), 5000000000) AS g,
  COALESCE((SELECT * FROM unnest('{}'::int[])), 2.5) AS empty,
  (SELECT avg(x) FROM (SELECT COALESCE((SELECT * FROM unnest('{1,4}'::int[]) LIMIT 1), i) AS x
     FROM t) q) AS a;
