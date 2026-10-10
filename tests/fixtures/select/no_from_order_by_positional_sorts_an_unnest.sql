-- A positional key over a no-FROM select binds the Nth **output** column and is kept as a
-- literal for `eval_order_key` to resolve by index (`plan_no_from_order_limit` checks only
-- its range). The row-expanding `unnest` is what makes the sort observable; the by-name
-- spelling is `select/no_from_order_by_sorts_an_unnest.sql`, and the out-of-range
-- polarity is `select/no_from_order_by_bad_position_loud.sql`.
-- @test
SELECT unnest(ARRAY[3, 1, 2]) AS q ORDER BY 1;
