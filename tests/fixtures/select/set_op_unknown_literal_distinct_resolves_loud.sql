-- `DISTINCT` resolves every unknown entry of its target list to `text`.
-- @expect-error: UNION types integer and text cannot be matched
-- @test
SELECT 1 UNION ALL SELECT DISTINCT '2';
