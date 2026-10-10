-- A recursive CTE with an `UPDATE` body and no data-modifying CTE: the recursive CTE
-- reads before anything is written, so running it first is the statement's snapshot.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3) UPDATE s SET x = x + 1000 FROM r WHERE x = n;
SELECT x FROM s ORDER BY x;
