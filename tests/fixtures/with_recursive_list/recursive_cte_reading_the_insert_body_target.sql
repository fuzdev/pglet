-- A recursive CTE seeded from the table the body inserts into reads the rows the
-- statement started with — the snapshot 18.6 reads it at — so its seed is the starting
-- maximum, never a row the body writes.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @test
WITH RECURSIVE r(n) AS (SELECT max(x) FROM s UNION ALL SELECT n + 1 FROM r WHERE n < 4)
INSERT INTO s SELECT n FROM r;
SELECT x FROM s ORDER BY x;
