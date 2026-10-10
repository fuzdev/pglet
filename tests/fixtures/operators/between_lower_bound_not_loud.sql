-- A `b_expr` has no prefix `NOT`: the refusal is the grammar's, ahead of any typing.
-- @test
-- @expect-error: syntax error at or near "NOT" (42601)
SELECT 1 BETWEEN NOT 0 AND 2;
