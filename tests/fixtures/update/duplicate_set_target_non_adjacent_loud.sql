-- The two assignments need not be adjacent — `rewriteTargetListIU` collects the whole
-- target list into a per-attribute array before it merges — so a repeat behind an
-- unrelated assignment is refused too. pglet wrote `a = 3, b = 2`.
-- @setup
CREATE TABLE dstn (a INT, b INT, c INT);
INSERT INTO dstn VALUES (0, 0, 0);

-- @expect-error: multiple assignments to same column "a" (42601)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dstn SET a = 1, b = 2, a = 3;
