-- `ANY ((SELECT …))` is the sub-select form on 18.6, which compares the operand with each
-- row's value, here a `text[]`; read as an array-valued scalar subquery it compared the
-- operand with each element instead. pglet does not support the sub-select form, and
-- refuses it where it is evaluated.
-- @expect-error: ANY over a sub-select is not supported
-- @test
SELECT '{1}' = ANY ((SELECT ARRAY['{1}'])) AS a;
