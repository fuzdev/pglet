-- A `UNION` chain with no self-reference under `RECURSIVE` is an ordinary CTE; it was
-- refused as a malformed recursive query.
-- @test
WITH RECURSIVE q AS (SELECT 1 AS z UNION SELECT 2 UNION SELECT 3)
SELECT z FROM q ORDER BY z;
