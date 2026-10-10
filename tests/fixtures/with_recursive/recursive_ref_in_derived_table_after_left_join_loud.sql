-- The control: under a `LEFT JOIN (` the derived table is the nullable side, so its
-- recursive reference is refused as within an outer join.
-- @setup
CREATE TABLE t1 (a INT, y TEXT);
-- @test
-- @expect-error: recursive reference to query "r" must not appear within an outer join
-- @expect-message
WITH RECURSIVE r(n) AS (
  SELECT 1
  UNION ALL
  SELECT s.n + 1 FROM t1 LEFT JOIN (SELECT * FROM r) s ON s.n = t1.a WHERE s.n < 3
) SELECT n FROM r;
