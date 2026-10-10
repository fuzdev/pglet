-- A `CHECK` holding `ANY ((SELECT …))` is 18.6's `0A000 cannot use subquery in check
-- constraint`. The refusal of the sub-select form is `0A000` too, so this holds the code, not
-- which of the two is raised; the `select/` `any_parenthesized_sub_select_*` fixtures hold the
-- order.
-- @expect-error: cannot use subquery in check constraint
-- @test
CREATE TABLE c9 (x INT CHECK (x = ANY ((SELECT ARRAY[1]))));
