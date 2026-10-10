-- The two claims on one column need not be adjacent: `checkInsertTargets` carries the
-- whole claimed set forward, so a repeat behind an unrelated name is refused too. pglet
-- wrote `a = 3, b = 2` here.
-- @setup
CREATE TABLE dtcn (a INT, b INT, c INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtcn (a, b, a) VALUES (1, 2, 3);
