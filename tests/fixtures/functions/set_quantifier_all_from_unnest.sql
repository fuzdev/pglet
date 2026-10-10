-- The **FROM-clause** `unnest(…)` is the one function call outside `parse_function_call`
-- a set quantifier can be written on (`parse_table_factor` dispatches a derived table,
-- this `unnest`, a CTE reference, a catalog view and a base table — nothing else), and
-- it is a second route to the same token: reading it nowhere left this spelling at
-- 42601 here against `1, 2` on 18.6, while the select-list spelling already agreed.
-- The accept polarity, so a fix that refuses both keywords at this route reddens here.
-- @test
SELECT * FROM unnest(ALL ARRAY[1, 2]) AS u(x);
