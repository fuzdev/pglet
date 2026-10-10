-- The right arm of a plain `EXCEPT` refuses a self-reference.
-- @expect-error: recursive reference within EXCEPT (42P19)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL (SELECT 5 EXCEPT SELECT n FROM r))
SELECT * FROM r;
