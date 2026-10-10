-- A later CTE named like a base table reads the recursive CTE, and shadows the table.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 2),
  s AS (SELECT * FROM r)
SELECT * FROM s ORDER BY 1;
