-- A self-reference inside `IN (SELECT …)` in the recursive term; it was answered.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: self-reference within a sub-link (42P19)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT x FROM s WHERE x IN (SELECT n + 1 FROM r))
SELECT * FROM r;
