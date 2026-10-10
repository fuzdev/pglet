-- A self-reference in a plain `INTERSECT`'s right arm is allowed as in its left.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT 2 INTERSECT SELECT n + 1 FROM r WHERE n < 3)
SELECT * FROM r;
