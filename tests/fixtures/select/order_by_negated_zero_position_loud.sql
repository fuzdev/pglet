-- `-0` folds to the constant `0`, and position `0` is out of range on a 1-based ordinal:
-- `42P10 ORDER BY position 0 is not in select list`, the same answer the bare `0`
-- spelling gives. The pair matters because the fold is what makes the *sign* reach the
-- bounds check at all — `-0` reads as an ordinal here and as an operator node without it.
-- @setup
CREATE TABLE obnz (x INT, y INT);
INSERT INTO obnz VALUES (3, 1), (1, 2), (2, 3);
-- @test
-- @expect-error: ORDER BY position 0 is not in select list (42P10)
SELECT x FROM obnz ORDER BY -0;
