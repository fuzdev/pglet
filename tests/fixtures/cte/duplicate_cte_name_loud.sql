-- `transformWithClause` scans the raw `WITH` list for name collisions before any body
-- is analysed. Last-wins here silently answered the second body's row.
-- @expect-error: two WITH queries under one name
-- @expect-message
-- @test
WITH c(x) AS (SELECT 1), c(y) AS (SELECT 2) SELECT * FROM c;
