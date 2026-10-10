-- An `EXCEPT ALL` closes its left arm as well as its right.
-- @expect-error: recursive reference within EXCEPT (42P19)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL (SELECT n + 1 FROM r WHERE n < 3 EXCEPT ALL SELECT 99))
SELECT * FROM r;
