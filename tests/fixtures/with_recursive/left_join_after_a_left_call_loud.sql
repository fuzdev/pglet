-- The control: a real `LEFT JOIN r` beside a `left(…)` call still makes the recursive
-- reference an outer join's nullable side, 42P19.
-- @setup
CREATE TABLE t1 (a INT, y TEXT);
-- @test
-- @expect-error: recursive reference to query "r" must not appear within an outer join
WITH RECURSIVE r(n) AS (
  SELECT 1
  UNION ALL
  SELECT r.n + 1 FROM t1 LEFT JOIN r ON left(t1.y, 1) = 'a' WHERE r.n < 3
) SELECT n FROM r;
