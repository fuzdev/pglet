-- A bare `NULL` is unknown the same way a string constant is, so a sort key naming it
-- resolves it to `text` too.
-- @expect-error: UNION types integer and text cannot be matched
-- @test
SELECT 1 UNION ALL (SELECT NULL ORDER BY 1);
