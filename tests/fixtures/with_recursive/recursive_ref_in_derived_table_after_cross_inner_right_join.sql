-- `CROSS JOIN (`, `INNER JOIN (` and `RIGHT JOIN (` open a derived table too; under a
-- `RIGHT JOIN` the derived table is the preserved side, so its recursive reference stands.
-- @setup
CREATE TABLE t1 (a INT, y TEXT);
INSERT INTO t1 VALUES (1, 'ab'), (2, 'cd');
-- @test
WITH RECURSIVE r(n) AS (
  SELECT 1
  UNION ALL
  SELECT s.n + 1 FROM t1 CROSS JOIN (SELECT * FROM r) s WHERE s.n < 3
), q(n) AS (
  SELECT 1
  UNION ALL
  SELECT s.n + 1 FROM t1 INNER JOIN (SELECT * FROM q) s ON s.n = t1.a WHERE s.n < 3
), w(n) AS (
  SELECT 1
  UNION ALL
  SELECT s.n + 1 FROM t1 RIGHT JOIN (SELECT * FROM w) s ON s.n = t1.a WHERE s.n < 3
) SELECT 'r' AS k, n FROM r UNION ALL SELECT 'q', n FROM q UNION ALL SELECT 'w', n FROM w ORDER BY 1, 2;
