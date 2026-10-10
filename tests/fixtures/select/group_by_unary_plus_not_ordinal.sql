-- `GROUP BY +1` is **not** an ordinal. Postgres's grammar folds unary `-` over a
-- numeric constant into the constant (`doNegate`, `gram.y`) but leaves `'+' a_expr` an
-- operator node, so `+1` is never an `A_Const` and `findTargetlistEntrySQL92` never
-- reads it as a position. It is an ordinary constant grouping key, which leaves the
-- projected `x` ungrouped: `42803`.
--
-- pglet's parser discarded unary `+` outright, making `+1` a bare literal and therefore
-- an ordinal — grouping by output column 1 and returning rows. That was the inverse of
-- Postgres in both directions; the minus half is closed too, and
-- `group_by_negative_position_loud.sql` pins it.
-- @setup
CREATE TABLE gbup (x INT, y INT);
INSERT INTO gbup VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: column "x" must appear in the GROUP BY clause (42803)
SELECT x FROM gbup GROUP BY +1;
