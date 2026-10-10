-- The second degrade shape: a `UNION` body whose recursive term never self-references
-- is an ordinary CTE, its column list applied the same way.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT 2) SELECT n FROM r ORDER BY n;
