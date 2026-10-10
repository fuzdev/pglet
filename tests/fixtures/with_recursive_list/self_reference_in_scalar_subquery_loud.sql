-- A self-reference inside an expression sub-link is `42P19 … within a subquery`.
-- This one was answered: the recursive term yields no row, so nothing ever read the
-- reference.
-- @setup
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (2);
-- @expect-error: self-reference within a sub-link (42P19)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT (SELECT max(n) FROM r) + 1 FROM s WHERE false)
SELECT * FROM r;
