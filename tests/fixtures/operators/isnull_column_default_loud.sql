-- A column `DEFAULT` is a `b_expr`, which has neither postfix test: `ISNULL` there is a
-- syntax error on both engines, where the parenthesized `a_expr` is accepted.
-- @test
-- @expect-error: syntax error at or near "ISNULL" (42601)
CREATE TABLE ind (a BOOL DEFAULT NULL ISNULL);
