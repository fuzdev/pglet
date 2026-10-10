-- A literal that ends inside an unquoted element: that element is never converted.
-- @test
-- @expect-error: the array reader's 22P02, not the element's 22007
SELECT '{x'::date[];
