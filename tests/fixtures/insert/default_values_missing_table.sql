-- The target relation still outranks everything: `DEFAULT VALUES` names no columns
-- and lowers to one empty row, and the executor's `42P01` is what the statement
-- reports — not a syntax error, and not a silent success over an absent table.

-- @expect-error: the missing relation is 42P01
-- @test
INSERT INTO no_such_default_values_target DEFAULT VALUES;
