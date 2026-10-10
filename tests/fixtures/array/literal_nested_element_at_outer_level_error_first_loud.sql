-- An element at the wrong depth is converted before its depth is refused: its own input
-- error comes ahead of the array reader's.
-- @test
-- @expect-error: the element's own 22007, not the array reader's 22P02
SELECT '{{2024-01-01},x}'::date[];
