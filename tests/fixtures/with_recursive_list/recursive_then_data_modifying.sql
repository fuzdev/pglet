-- A recursive CTE beside a data-modifying CTE that reads it: the recursive CTE
-- materializes first, the `INSERT` writes its rows once, and the second statement sees
-- the write.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  w AS (INSERT INTO s SELECT n FROM r RETURNING x)
SELECT count(*) FROM w;
SELECT x FROM s ORDER BY x;
