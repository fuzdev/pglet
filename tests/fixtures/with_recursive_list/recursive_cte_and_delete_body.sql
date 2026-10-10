-- A recursive CTE with a `DELETE` body and no data-modifying CTE.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3) DELETE FROM s WHERE x IN (SELECT n FROM r);
SELECT count(*) FROM s;
