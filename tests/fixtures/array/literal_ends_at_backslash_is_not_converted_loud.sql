-- A literal that ends at a backslash, with no closing brace: the element it cuts short
-- is never converted, so the refusal is the array reader's.
-- @test
-- @expect-error: the array reader's 22P02, not the element's 22007
SELECT '{x\'::date[];
