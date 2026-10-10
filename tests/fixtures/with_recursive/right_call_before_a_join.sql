-- A `right(…)` call followed by a `JOIN` in a recursive body is a function, not a
-- `RIGHT JOIN`: it makes no relation before it nullable.
-- @setup
CREATE TABLE t1 (a INT, y TEXT);
CREATE TABLE t2 (a INT, y TEXT);
INSERT INTO t1 VALUES (1, 'ab'), (2, 'cd');
INSERT INTO t2 VALUES (1, 'ab'), (3, 'xy');
-- @test
WITH RECURSIVE r(n) AS (
  SELECT 1
  UNION ALL
  SELECT r.n + 1 FROM r JOIN t1 ON t1.y = right(t1.y, 2) JOIN t2 ON t2.a = r.n WHERE r.n < 3
) SELECT n FROM r ORDER BY n;
