-- The mirror of `select/set_op_nested_all_unknown_is_text_loud.sql`: the nested operand on
-- the right resolves to `integer` (its `'2'` takes `3`'s type), and the leading `'1'`
-- takes that.
-- @test
SELECT '1' AS v UNION (SELECT '2' UNION SELECT 3) ORDER BY 1;
