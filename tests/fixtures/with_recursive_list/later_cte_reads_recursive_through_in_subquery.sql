-- A later CTE reading an earlier recursive CTE through `IN (SELECT …)`.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT x FROM s WHERE x IN (SELECT n FROM r))
SELECT x FROM q ORDER BY x;
