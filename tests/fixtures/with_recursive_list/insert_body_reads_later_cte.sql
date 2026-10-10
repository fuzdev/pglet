-- A data-modifying statement body reading a CTE that reads the recursive one. The
-- body is dispatched as a statement, so the write runs with the recursive CTE bound;
-- the second statement reads what it wrote.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT * FROM r)
INSERT INTO s SELECT n FROM q;
SELECT x FROM s ORDER BY x;
