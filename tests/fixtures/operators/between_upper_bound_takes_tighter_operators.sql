-- The control: an operator binding tighter than `BETWEEN` stays in the upper bound, and
-- a boolean `AND` after the bound is the outer connective.
-- @test
SELECT 'b' BETWEEN 'a' AND 'c' || 'x' AS a, 1 BETWEEN 0 AND 1 + 1 AS b,
       1 BETWEEN 0 AND 2 AND true AS c, NOT 1 BETWEEN 0 AND 2 AS d;
