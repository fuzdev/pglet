-- Two further pairs of parentheses around a body are the same query too.
-- @test
WITH RECURSIVE r(n) AS (((SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3)))
SELECT * FROM r;
