-- `RETURNING` does not save the statement: `checkInsertTargets` runs while the row
-- source is transformed and `transformReturningClause` after it, so the column list's
-- `42701` outranks anything the clause says — including an unknown name in it, which is
-- `42703` when the list itself is clean. pglet wrote the row and returned `2`.
-- @setup
CREATE TABLE dtcr (a INT, b INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtcr (a, a) VALUES (1, 2) RETURNING a;
