-- A bare `NULL` and a string constant are both unknown: the leading `1` makes the first
-- set operation `integer`, and the `'2'` takes that type from it.
-- @test
SELECT 1 AS v UNION ALL SELECT NULL UNION ALL SELECT '2' ORDER BY 1;
