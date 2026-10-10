-- An `ORDER BY` key naming the constant resolves it to `text` before the set operation
-- sees it, as it does for an `INSERT … SELECT` source.
-- @expect-error: UNION types integer and text cannot be matched
-- @test
SELECT 1 UNION ALL (SELECT '2' ORDER BY 1);
