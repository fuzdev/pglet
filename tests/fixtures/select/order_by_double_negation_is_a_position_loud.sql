-- `doNegate` folds an already-folded constant again, so `- -99` is the **positive**
-- ordinal `99` and not a doubly-wrapped operator node: `42P10 position 99`. This is the
-- spelling that separates "fold the operand when it is a literal" from "fold only a
-- literal written directly after the minus" — the second reading leaves `- -99` an
-- expression, which both engines would then run.
-- @setup
CREATE TABLE obdn (x INT, y INT);
INSERT INTO obdn VALUES (3, 1), (1, 2), (2, 3);
-- @test
-- @expect-error: ORDER BY position 99 is not in select list (42P10)
SELECT x FROM obdn ORDER BY - -99;
