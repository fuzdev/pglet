-- A `b_expr` has no `IS [NOT] NULL` either.
-- @test
-- @expect-error: syntax error at or near "NULL" (42601)
SELECT true BETWEEN NULL IS NULL AND true;
