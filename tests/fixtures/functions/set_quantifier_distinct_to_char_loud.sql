-- The binary-modeled arm (`binary_call`, shared with `starts_with`), which builds a
-- `BinaryOp` rather than a function node and so drops the quantifier by a different
-- route than the string and math arms. 42809 on both; pglet answered `2026`.
-- @test
-- @expect-error: DISTINCT specified, but to_char is not an aggregate function
SELECT to_char(DISTINCT now(), 'YYYY');
