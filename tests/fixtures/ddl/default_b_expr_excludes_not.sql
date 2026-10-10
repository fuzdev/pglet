-- The other direction of the same grammar: `b_expr` has no `NOT` production, so
-- `DEFAULT NOT true` is a syntax error at the `NOT` itself — before the operand is
-- looked at. The `SET DEFAULT` control
-- (`alter_column_set_default_a_expr_admits_not.sql`) takes full `a_expr` and accepts
-- the identical expression, which is what identifies the grammar rather than the
-- expression.
-- @expect-error: NOT is an a_expr production, excluded from a column constraint's DEFAULT
-- @test
CREATE TABLE def_not (a INT, b BOOL DEFAULT NOT true);
