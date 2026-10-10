-- A `BETWEEN`'s lower bound is a `b_expr`, which has no postfix null test.
-- @test
-- @expect-error: syntax error at or near "NOTNULL" (42601)
SELECT true BETWEEN NULL NOTNULL AND true;
