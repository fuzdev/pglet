-- A plain `EXCEPT` leaves its left arm open: the self-reference there is allowed.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL (SELECT n + 1 FROM r WHERE n < 3 EXCEPT SELECT 99))
SELECT * FROM r;
