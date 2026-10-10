-- Both sides of a `FULL JOIN` are nullable.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: self-reference within an outer join (42P19)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT s.x FROM s FULL OUTER JOIN r ON r.n = s.x)
SELECT * FROM r;
