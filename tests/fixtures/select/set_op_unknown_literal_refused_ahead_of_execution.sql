-- The conversion of an unknown constant is a parse-analysis error, so it outranks an
-- error the other side raises only when it executes.
-- @expect-error: invalid input syntax for type integer
-- @expect-message
-- @test
SELECT 1 / 0 UNION ALL SELECT 'x';
