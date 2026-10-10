-- Each element is converted as it is read: the element a closing brace ends is converted,
-- and its own input error comes ahead of the junk after that brace.
-- @test
-- @expect-error: the element's own 22007, not the array reader's 22P02
SELECT '{x}2024-01-02}'::date[];
