-- Every right-hand side of one `SET` list reads the target row, never an earlier
-- assignment's result: `b = a` reads the old `a` (10), not the `11` the assignment to its
-- left computes. pglet answered `11 | 11`.
-- @setup
CREATE TABLE sor (a INT, b INT);
INSERT INTO sor VALUES (10, 0);

-- @test
UPDATE sor SET a = a + 1, b = a RETURNING a, b;
