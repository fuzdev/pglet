-- The control: a real `RIGHT JOIN` after a `right(…)` call still makes the recursive
-- reference before it nullable, 42P19.
-- @setup
CREATE TABLE t1 (a INT, y TEXT);
-- @test
-- @expect-error: recursive reference to query "r" must not appear within an outer join
WITH RECURSIVE r(n) AS (
  SELECT 1
  UNION ALL
  SELECT r.n + 1 FROM r JOIN t1 ON right(t1.y, 1) = 'a' RIGHT JOIN t1 t3 ON true WHERE r.n < 3
) SELECT n FROM r;
