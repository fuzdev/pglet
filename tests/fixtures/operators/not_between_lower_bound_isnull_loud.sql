-- The `NOT BETWEEN` spelling reads its lower bound as a `b_expr` too.
-- @test
-- @expect-error: syntax error at or near "ISNULL" (42601)
SELECT 1 NOT BETWEEN NULL ISNULL AND 2;
