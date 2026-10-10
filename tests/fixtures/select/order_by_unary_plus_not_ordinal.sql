-- The same unary-plus rule as `group_by_unary_plus_not_ordinal.sql`, in the clause
-- where it changes **values** rather than an error code: `ORDER BY +1` sorts by a
-- constant, so every row compares equal and the input order survives. Sorting by
-- output column 1 would reorder the rows.
--
-- The seed data is deliberately not already in `x` order, which is what makes the two
-- readings distinguishable — with sorted input this fixture would pass either way.
-- pglet read `+1` as an ordinal and returned `1,2,3`; Postgres returns `3,1,2`.
-- @setup
CREATE TABLE obup (x INT, y INT);
INSERT INTO obup VALUES (3, 1), (1, 2), (2, 3);
-- @test
SELECT x, y FROM obup ORDER BY +1;
