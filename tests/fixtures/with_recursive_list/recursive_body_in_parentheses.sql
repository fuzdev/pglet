-- A body wrapped in a further pair of parentheses is the same query.
-- @test
WITH RECURSIVE r(n) AS ((SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3))
SELECT * FROM r;
