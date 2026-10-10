-- An earlier element's input error comes ahead of a later structural error.
-- @test
-- @expect-error: the first element's own 22007, not the array reader's 22P02
SELECT '{x,,01:00}'::time[];
