-- `-1` is an ordinal, a negative one: `parser::expr`'s `negate` folds the sign into the
-- literal (`doNegate`), so the key reaches the bounds check as `position -1`. The fold runs
-- in the **parser**, over every `ORDER BY` key this arm parses, and **this arm still
-- answered a row** — because the folded key was then discarded with the rest of the
-- no-FROM tail, ahead of any check that could read it. The fold was never the hole, which
-- is what the exception clause on its own doc recorded and what routing the tail retires.
-- @expect-error: ORDER BY position -1 is not in select list (42P10)
-- @test
SELECT 1 AS q ORDER BY -1;
