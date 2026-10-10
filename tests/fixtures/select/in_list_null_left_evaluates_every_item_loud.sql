-- A NULL left operand of an `IN` list is NULL whatever the items, but every item is still
-- evaluated: the second item's division raises on both engines.
-- @setup
CREATE TABLE tn (x INT, y INT);
INSERT INTO tn VALUES (NULL, 0);
-- @expect-error: division by zero
-- @test
SELECT x IN (1, 1/y) FROM tn;
