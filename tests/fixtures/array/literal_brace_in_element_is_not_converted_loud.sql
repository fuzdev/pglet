-- An element cut short by an opening brace is never converted: the refusal is the array
-- reader's, not the input error the element's text so far would raise.
-- @test
-- @expect-error: the array reader's 22P02, not the element's 22007
SELECT '{x{}'::date[];
