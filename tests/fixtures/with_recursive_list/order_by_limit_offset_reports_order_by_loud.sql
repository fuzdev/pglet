-- Several decorations: 18.6 tests `ORDER BY` first, whatever the text order.
-- @expect-error: OFFSET is tested before LIMIT (0A000)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 LIMIT 2 OFFSET 1)
SELECT * FROM r;
