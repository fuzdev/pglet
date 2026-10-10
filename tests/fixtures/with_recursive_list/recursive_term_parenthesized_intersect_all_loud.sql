-- A parenthesized `INTERSECT ALL` in the recursive term refuses the self-reference too.
-- @expect-error: recursive reference within INTERSECT (42P19)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL (SELECT n + 1 FROM r WHERE n < 3 INTERSECT ALL SELECT 2))
SELECT * FROM r;
