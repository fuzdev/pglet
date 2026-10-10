-- A `JOIN (` opens a derived table, not a call: the recursive reference inside it is
-- a FROM item of the join, neither a subquery's nor an outer join's.
-- @setup
CREATE TABLE t1 (a INT, y TEXT);
INSERT INTO t1 VALUES (1, 'ab'), (2, 'cd');
-- @test
WITH RECURSIVE r(n) AS (
  SELECT 1
  UNION ALL
  SELECT s.n + 1 FROM t1 JOIN (SELECT * FROM r) s ON s.n = t1.a WHERE s.n < 3
) SELECT n FROM r ORDER BY n;
