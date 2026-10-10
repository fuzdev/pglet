-- The accept control for routing the no-FROM tail: the `ORDER BY` is not merely *validated*
-- but actually applied. A projection `unnest` is what makes that observable — a one-row
-- `Values` sorts identically however the clause is treated, so a fixture over one row
-- would pass with the clause still discarded.
--
-- Its positional twin is `select/no_from_order_by_positional_sorts_an_unnest.sql`.
-- @test
SELECT unnest(ARRAY[3, 1, 2]) AS q ORDER BY q;
