-- A NULL array expands to no row, and its volatile sibling is still evaluated, as at
-- 18.6's first ProjectSet call: the division fails.
-- @test
-- @expect-error: division by zero
SELECT unnest(NULL::int[]), random() + 1/0;
