-- The reverse order: a structural error ahead of an element that would be refused is the
-- array reader's, since that element is never reached.
-- @test
-- @expect-error: the array reader's 22P02, not the last element's 22007
SELECT '{2024-01-01,,x}'::date[];
