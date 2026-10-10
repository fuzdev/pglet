-- Control: the left side of a `LEFT JOIN` is not nullable.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT s.x FROM r LEFT JOIN s ON r.n + 1 = s.x WHERE s.x IS NOT NULL)
SELECT n FROM r ORDER BY n;
