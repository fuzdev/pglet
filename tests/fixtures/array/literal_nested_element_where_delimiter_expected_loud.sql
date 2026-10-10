-- An element directly after a sub-array's closing brace stands where a delimiter is
-- expected: the refusal is the array reader's, ahead of that element's own input error.
-- @test
-- @expect-error: the array reader's 22P02, not the element's 22007
SELECT '{{2024-01-01}x}'::date[];
