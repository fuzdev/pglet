-- A set operation in a CTE unifies the same way; `n + 1` is integer arithmetic.
-- @test
WITH w AS (SELECT 1 AS n UNION ALL SELECT '2') SELECT n + 1 AS m FROM w ORDER BY 1;
